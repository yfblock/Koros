//! ext2 inode reading, writing, and block pointer resolution.
//!
// allow: SIZE_OK — ext2 inode handling requires block resolution (direct + 3
// levels of indirect), allocation, read-modify-write, and disk persistence.
// Splitting would scatter tightly-coupled logic across files for no clarity gain.

extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::fs::{FsError, FileType, INode, Metadata};

use super::xattr;
use super::{Ext2Fs, EXT2_ROOT_INO};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Number of direct block pointers in an inode.
const DIRECT_BLOCKS: usize = 12;

// ext2 file-type constants (upper 4 bits of i_mode)
const EXT2_S_IFMT: u16 = 0xF000;
const EXT2_S_IFSOCK: u16 = 0xC000;
const EXT2_S_IFLNK: u16 = 0xA000;
const EXT2_S_IFREG: u16 = 0x8000;
const EXT2_S_IFBLK: u16 = 0x6000;
const EXT2_S_IFDIR: u16 = 0x4000;
const EXT2_S_IFCHR: u16 = 0x2000;
const EXT2_S_IFIFO: u16 = 0x1000;

/// A symlink target of at most this many bytes is stored inline in the
/// `i_block` array ("fast symlink") rather than in a data block.
const FAST_SYMLINK_MAX: usize = 60;

/// `i_flags` bit marking a directory as hash-tree (htree / `dir_index`)
/// indexed.  Koros only manipulates directories linearly, so it clears this
/// flag before modifying an indexed directory (a safe "downgrade").
const EXT2_INDEX_FL: u32 = 0x0000_1000;

/// Magic number at the start of an ext2 extended-attribute block header.
const EXT2_XATTR_MAGIC: u32 = 0xEA02_0000;
/// Byte offset of `h_refcount` within the xattr block header.
const XATTR_REFCOUNT_OFFSET: usize = 4;

// ---------------------------------------------------------------------------
// On-disk inode structure (128 bytes, matches ext2 specification)
// ---------------------------------------------------------------------------

/// Raw on-disk ext2 inode, byte-for-byte compatible with the specification.
///
/// The base inode is 128 bytes.  Filesystems with `inode_size > 128` store
/// extended attributes in the extra space, which we ignore for now.
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct RawInode {
    /// File mode (type + permissions).
    pub i_mode: u16,
    /// Owner user ID.
    pub i_uid: u16,
    /// Lower 32 bits of file size in bytes.
    pub i_size: u32,
    /// Last access time (POSIX timestamp).
    pub i_atime: u32,
    /// Creation / status-change time (POSIX timestamp).
    pub i_ctime: u32,
    /// Last modification time (POSIX timestamp).
    pub i_mtime: u32,
    /// Deletion time (POSIX timestamp; 0 if not deleted).
    pub i_dtime: u32,
    /// Owner group ID.
    pub i_gid: u16,
    /// Hard-link count.
    pub i_links_count: u16,
    /// Number of 512-byte blocks reserved for this inode.
    pub i_blocks: u32,
    /// File flags (ext2-specific).
    pub i_flags: u32,
    /// OS-dependent value #1.
    pub i_osd1: u32,
    /// Block pointers: 12 direct, 1 indirect, 1 double, 1 triple.
    pub i_block: [u32; 15],
    /// File version (used by NFS).
    pub i_generation: u32,
    /// Extended attribute block (ACL).
    pub i_file_acl: u32,
    /// Directory ACL / upper 32 bits of file size (for regular files).
    pub i_dir_acl: u32,
    /// Fragment address (not used on most systems).
    pub i_faddr: u32,
    /// OS-dependent value #2.
    pub i_osd2: [u8; 12],
}

// Compile-time assertion: exactly 128 bytes.
const _: () = assert!(core::mem::size_of::<RawInode>() == 128);

// ---------------------------------------------------------------------------
// Ext2INode — a parsed inode bound to its filesystem
// ---------------------------------------------------------------------------

/// An ext2 inode with a reference to its parent filesystem.
///
/// The raw inode is wrapped in a [`spin::Mutex`] to provide interior
/// mutability — required because [`INode::write_at`] and [`INode::setattr`]
/// take `&self` while needing to update on-disk inode fields.
pub struct Ext2INode {
    /// The filesystem this inode belongs to.
    fs: Arc<Ext2Fs>,
    /// The inode number (1-based).
    inode_num: u32,
    /// The raw on-disk inode data (protected by spinlock for write operations).
    raw: spin::Mutex<RawInode>,
}

impl Ext2INode {
    /// Read inode `inode_num` from disk.
    ///
    /// Calculates which block group contains the inode, locates the inode
    /// table block, and reads the [`RawInode`] structure.
    pub fn read(fs: &Arc<Ext2Fs>, inode_num: u32) -> Result<Self, FsError> {
        if inode_num == 0 {
            return Err(FsError::InvalidInput);
        }

        let sb = fs.super_block();
        let inodes_per_group = sb.inodes_per_group;
        let inode_size = sb.inode_size as usize;

        // Inodes are 1-indexed; find the group and the index within the group.
        let group = (inode_num - 1) / inodes_per_group;
        let index = (inode_num - 1) % inodes_per_group;

        // Locate the inode table for this block group.
        let groups = fs.block_groups();
        let bg = groups
            .get(group as usize)
            .ok_or(FsError::InvalidInput)?;
        let inode_table_block = bg.bg_inode_table as usize;

        // Byte offset of this inode within the inode table.
        let offset_in_table = (index as usize) * inode_size;

        // Which filesystem block contains this inode?
        let block_size = fs.block_size();
        let fs_block = inode_table_block + offset_in_table / block_size;
        let offset_in_block = offset_in_table % block_size;

        // Read the filesystem block from the device.
        let mut block_buf = vec![0u8; block_size];
        read_fs_block(fs, fs_block, &mut block_buf)?;

        // Reinterpret the inode bytes.
        // inode_size may be > 128 but we only need the standard 128 bytes.
        let raw: &RawInode = bytemuck_ref(&block_buf[offset_in_block..]);

        Ok(Self {
            fs: Arc::clone(fs),
            inode_num,
            raw: spin::Mutex::new(*raw),
        })
    }

    /// The inode number (1-based).
    pub fn inode_num(&self) -> u32 {
        self.inode_num
    }

    /// A reference to the parent filesystem.
    pub fn fs(&self) -> &Arc<Ext2Fs> {
        &self.fs
    }

    /// A guard to the raw on-disk inode.
    ///
    /// Returns a [`spin::MutexGuard`] so callers can read (and, internally,
    /// write) inode fields through auto-deref.
    pub fn raw(&self) -> spin::MutexGuard<'_, RawInode> {
        self.raw.lock()
    }

    /// Resolve a logical block number to a physical block number.
    ///
    /// Returns `0` for sparse (unallocated) blocks.
    ///
    /// Block-pointer layout in `i_block[15]`:
    /// - `[0..12]`  — 12 direct pointers
    /// - `[12]`     — single indirect pointer
    /// - `[13]`     — double indirect pointer
    /// - `[14]`     — triple indirect pointer
    pub fn resolve_block_id(&self, logical_block: u32) -> Result<u32, FsError> {
        let block_size = self.fs.block_size();
        let ptrs_per_block = (block_size / core::mem::size_of::<u32>()) as u32;

        let direct_limit = DIRECT_BLOCKS as u32;
        let indirect_limit = direct_limit + ptrs_per_block;
        let double_limit = indirect_limit + ptrs_per_block * ptrs_per_block;

        if logical_block < direct_limit {
            // --- Direct block ---
            let raw = self.raw.lock();
            Ok(raw.i_block[logical_block as usize])
        } else if logical_block < indirect_limit {
            // --- Single indirect ---
            let indirect_block = {
                let raw = self.raw.lock();
                raw.i_block[12]
            };
            if indirect_block == 0 {
                return Ok(0);
            }
            let offset = logical_block - direct_limit;
            self.read_indirect_entry(indirect_block, offset)
        } else if logical_block < double_limit {
            // --- Double indirect ---
            let double_block = {
                let raw = self.raw.lock();
                raw.i_block[13]
            };
            if double_block == 0 {
                return Ok(0);
            }
            let remaining = logical_block - indirect_limit;
            let first_idx = remaining / ptrs_per_block;
            let second_idx = remaining % ptrs_per_block;
            let indirect_block = self.read_indirect_entry(double_block, first_idx)?;
            if indirect_block == 0 {
                return Ok(0);
            }
            self.read_indirect_entry(indirect_block, second_idx)
        } else {
            // --- Triple indirect ---
            let triple_block = {
                let raw = self.raw.lock();
                raw.i_block[14]
            };
            if triple_block == 0 {
                return Ok(0);
            }
            let remaining = logical_block - double_limit;
            let p2 = ptrs_per_block * ptrs_per_block;
            let first_idx = remaining / p2;
            let second_idx = (remaining / ptrs_per_block) % ptrs_per_block;
            let third_idx = remaining % ptrs_per_block;

            let double_block = self.read_indirect_entry(triple_block, first_idx)?;
            if double_block == 0 {
                return Ok(0);
            }
            let indirect_block = self.read_indirect_entry(double_block, second_idx)?;
            if indirect_block == 0 {
                return Ok(0);
            }
            self.read_indirect_entry(indirect_block, third_idx)
        }
    }

    /// Read a single `u32` block pointer from an indirect block at `index`.
    fn read_indirect_entry(&self, block_id: u32, index: u32) -> Result<u32, FsError> {
        let block_size = self.fs.block_size();
        let mut buf = vec![0u8; block_size];
        read_fs_block(&self.fs, block_id as usize, &mut buf)?;

        let byte_offset = (index as usize) * core::mem::size_of::<u32>();
        if byte_offset + 4 > block_size {
            return Err(FsError::InvalidInput);
        }

        Ok(u32::from_le_bytes([
            buf[byte_offset],
            buf[byte_offset + 1],
            buf[byte_offset + 2],
            buf[byte_offset + 3],
        ]))
    }

    /// Write a single `u32` block pointer into an indirect block at `index`.
    fn write_indirect_entry(
        &self,
        block_id: u32,
        index: u32,
        value: u32,
    ) -> Result<(), FsError> {
        let block_size = self.fs.block_size();
        let mut buf = vec![0u8; block_size];
        read_fs_block(&self.fs, block_id as usize, &mut buf)?;

        let byte_offset = (index as usize) * core::mem::size_of::<u32>();
        if byte_offset + 4 > block_size {
            return Err(FsError::InvalidInput);
        }

        let bytes = value.to_le_bytes();
        buf[byte_offset..byte_offset + 4].copy_from_slice(&bytes);

        write_fs_block(&self.fs, block_id as usize, &buf)
    }

    /// Read an entire indirect block and return all block pointers.
    pub fn read_indirect_block(&self, block_id: u32) -> Result<Vec<u32>, FsError> {
        let block_size = self.fs.block_size();
        let mut buf = vec![0u8; block_size];
        read_fs_block(&self.fs, block_id as usize, &mut buf)?;

        let count = block_size / core::mem::size_of::<u32>();
        let mut pointers = Vec::with_capacity(count);
        for i in 0..count {
            let off = i * 4;
            pointers.push(u32::from_le_bytes([
                buf[off],
                buf[off + 1],
                buf[off + 2],
                buf[off + 3],
            ]));
        }
        Ok(pointers)
    }

    // -----------------------------------------------------------------------
    // Write-path helpers
    // -----------------------------------------------------------------------

    /// Ensure a physical block exists for `logical_block`, allocating one
    /// (and any necessary indirect blocks) if it does not.
    ///
    /// If the block is already mapped, returns its existing physical number.
    /// Otherwise allocates a new data block, wires it into the inode's block
    /// pointer tree, and returns the new physical block number.
    fn allocate_block_for_offset(&self, logical_block: u32) -> Result<(u32, u32), FsError> {
        // Fast path: block already exists (no new blocks allocated).
        let existing = self.resolve_block_id(logical_block)?;
        if existing != 0 {
            return Ok((existing, 0));
        }

        let block_size = self.fs.block_size();
        let ptrs_per_block = (block_size / core::mem::size_of::<u32>()) as u32;

        let direct_limit = DIRECT_BLOCKS as u32;
        let indirect_limit = direct_limit + ptrs_per_block;
        let double_limit = indirect_limit + ptrs_per_block * ptrs_per_block;

        // Allocate and zero the data block, preferring this inode's own group.
        let new_block = self.fs.alloc_block_goal(self.block_group())?;
        let zero_buf = vec![0u8; block_size];
        if let Err(e) = write_fs_block(&self.fs, new_block as usize, &zero_buf) {
            // Best-effort free on failure.
            let _ = self.fs.free_block(new_block);
            return Err(e);
        }
        // Count of filesystem blocks newly allocated during this call (the
        // data block plus any indirect metadata blocks), so the caller can
        // update `i_blocks` incrementally instead of rescanning the tree.
        let mut newly = 1u32;

        if logical_block < direct_limit {
            // --- Direct block ---
            let mut raw = self.raw.lock();
            raw.i_block[logical_block as usize] = new_block;
            return Ok((new_block, newly));
        }

        if logical_block < indirect_limit {
            // --- Single indirect ---
            let offset = logical_block - direct_limit;
            let indirect_block = {
                let mut raw = self.raw.lock();
                let ib = raw.i_block[12];
                if ib == 0 {
                    let allocated = self.alloc_and_zero_block()?;
                    newly += 1;
                    raw.i_block[12] = allocated;
                    allocated
                } else {
                    ib
                }
            };
            self.write_indirect_entry(indirect_block, offset, new_block)?;
            return Ok((new_block, newly));
        }

        if logical_block < double_limit {
            // --- Double indirect ---
            let remaining = logical_block - indirect_limit;
            let first_idx = remaining / ptrs_per_block;
            let second_idx = remaining % ptrs_per_block;

            let l1_block = {
                let mut raw = self.raw.lock();
                let b = raw.i_block[13];
                if b == 0 {
                    let allocated = self.alloc_and_zero_block()?;
                    newly += 1;
                    raw.i_block[13] = allocated;
                    allocated
                } else {
                    b
                }
            };

            let l2_block = {
                let entry = self.read_indirect_entry(l1_block, first_idx)?;
                if entry == 0 {
                    let allocated = self.alloc_and_zero_block()?;
                    newly += 1;
                    self.write_indirect_entry(l1_block, first_idx, allocated)?;
                    allocated
                } else {
                    entry
                }
            };

            self.write_indirect_entry(l2_block, second_idx, new_block)?;
            return Ok((new_block, newly));
        }

        // --- Triple indirect ---
        let remaining = logical_block - double_limit;
        let p2 = ptrs_per_block * ptrs_per_block;
        let first_idx = remaining / p2;
        let second_idx = (remaining / ptrs_per_block) % ptrs_per_block;
        let third_idx = remaining % ptrs_per_block;

        let l1_block = {
            let mut raw = self.raw.lock();
            let b = raw.i_block[14];
            if b == 0 {
                let allocated = self.alloc_and_zero_block()?;
                newly += 1;
                raw.i_block[14] = allocated;
                allocated
            } else {
                b
            }
        };

        let l2_block = {
            let entry = self.read_indirect_entry(l1_block, first_idx)?;
            if entry == 0 {
                let allocated = self.alloc_and_zero_block()?;
                newly += 1;
                self.write_indirect_entry(l1_block, first_idx, allocated)?;
                allocated
            } else {
                entry
            }
        };

        let l3_block = {
            let entry = self.read_indirect_entry(l2_block, second_idx)?;
            if entry == 0 {
                let allocated = self.alloc_and_zero_block()?;
                newly += 1;
                self.write_indirect_entry(l2_block, second_idx, allocated)?;
                allocated
            } else {
                entry
            }
        };

        self.write_indirect_entry(l3_block, third_idx, new_block)?;
        Ok((new_block, newly))
    }

    /// The block group this inode lives in (used as an allocation goal for
    /// data-block locality).
    fn block_group(&self) -> u32 {
        let inodes_per_group = self.fs.super_block().inodes_per_group;
        if inodes_per_group == 0 {
            0
        } else {
            (self.inode_num - 1) / inodes_per_group
        }
    }

    /// Allocate a filesystem block and zero it.  On failure the block is
    /// freed before returning the error.
    fn alloc_and_zero_block(&self) -> Result<u32, FsError> {
        let block = self.fs.alloc_block_goal(self.block_group())?;
        let zero_buf = vec![0u8; self.fs.block_size()];
        if let Err(e) = write_fs_block(&self.fs, block as usize, &zero_buf) {
            let _ = self.fs.free_block(block);
            return Err(e);
        }
        Ok(block)
    }

    /// Persist the in-memory [`RawInode`] back to its location on disk.
    ///
    /// Performs a read-modify-write of the inode table block because
    /// multiple inodes share a single filesystem block.
    fn write_inode_to_disk(&self) -> Result<(), FsError> {
        // Copy the raw inode out of the guard (128 bytes, Copy).
        let raw_copy = {
            let raw = self.raw.lock();
            *raw
        };

        let sb = self.fs.super_block();
        let inodes_per_group = sb.inodes_per_group;
        let inode_size = sb.inode_size as usize;
        drop(sb);

        let group = (self.inode_num - 1) / inodes_per_group;
        let index = (self.inode_num - 1) % inodes_per_group;

        let groups = self.fs.block_groups();
        let bg = groups
            .get(group as usize)
            .ok_or(FsError::InvalidInput)?;
        let inode_table_block = bg.bg_inode_table as usize;
        drop(groups);

        let offset_in_table = (index as usize) * inode_size;
        let block_size = self.fs.block_size();
        let fs_block = inode_table_block + offset_in_table / block_size;
        let offset_in_block = offset_in_table % block_size;

        // Read the inode table block (contains other inodes too).
        let mut block_buf = vec![0u8; block_size];
        read_fs_block(&self.fs, fs_block, &mut block_buf)?;

        // Overwrite just our inode's bytes.
        // SAFETY: `RawInode` is `repr(C, packed)` (alignment 1) and exactly
        // 128 bytes.  The cast is valid for packed types.
        let raw_bytes: &[u8] = unsafe {
            core::slice::from_raw_parts(
                (&raw_copy as *const RawInode).cast::<u8>(),
                core::mem::size_of::<RawInode>(),
            )
        };
        block_buf[offset_in_block..offset_in_block + core::mem::size_of::<RawInode>()]
            .copy_from_slice(raw_bytes);

        write_fs_block(&self.fs, fs_block, &block_buf)
    }

    // -----------------------------------------------------------------------
    // Size accounting (honours the `large_file` ro_compat feature)
    // -----------------------------------------------------------------------

    /// The logical file size in bytes.
    ///
    /// For regular files on a `large_file`-enabled filesystem the upper 32
    /// bits live in `i_dir_acl`; all other inodes use only `i_size`.
    pub fn size(&self) -> u64 {
        let raw = self.raw.lock();
        let low = raw.i_size as u64;
        if (raw.i_mode & EXT2_S_IFMT) == EXT2_S_IFREG && self.fs.has_large_file() {
            let high = raw.i_dir_acl as u64;
            (high << 32) | low
        } else {
            low
        }
    }

    /// Store `size` into the inode, splitting into `i_size` / `i_dir_acl` when
    /// the `large_file` feature applies.  Caller holds no lock.
    fn set_size(&self, size: u64) {
        let mut raw = self.raw.lock();
        raw.i_size = (size & 0xFFFF_FFFF) as u32;
        if (raw.i_mode & EXT2_S_IFMT) == EXT2_S_IFREG && self.fs.has_large_file() {
            raw.i_dir_acl = (size >> 32) as u32;
        }
    }

    /// Number of 512-byte sectors backing one filesystem block.
    fn sectors_per_block(&self) -> u32 {
        (self.fs.block_size() / 512) as u32
    }

    /// Recompute `i_blocks` by counting every allocated data block plus the
    /// indirect metadata blocks.
    fn recompute_i_blocks(&self) -> Result<(), FsError> {
        let (direct, file_acl) = {
            let raw = self.raw.lock();
            (raw.i_block, raw.i_file_acl)
        };
        let mut blocks = 0u32;
        for &b in direct.iter().take(DIRECT_BLOCKS) {
            if b != 0 {
                blocks += 1;
            }
        }
        blocks += self.count_indirect(direct[12], 1)?;
        blocks += self.count_indirect(direct[13], 2)?;
        blocks += self.count_indirect(direct[14], 3)?;
        // ext2 accounts the external extended-attribute block in i_blocks too.
        if file_acl != 0 {
            blocks += 1;
        }

        let sectors = blocks * self.sectors_per_block();
        self.raw.lock().i_blocks = sectors;
        Ok(())
    }

    /// Count allocated blocks reachable through an indirect pointer tree,
    /// including the indirect blocks themselves.
    fn count_indirect(&self, block: u32, level: u8) -> Result<u32, FsError> {
        if block == 0 {
            return Ok(0);
        }
        let ptrs = self.read_indirect_block(block)?;
        let mut count = 1; // the indirect block itself
        for &p in &ptrs {
            if p == 0 {
                continue;
            }
            if level == 1 {
                count += 1;
            } else {
                count += self.count_indirect(p, level - 1)?;
            }
        }
        Ok(count)
    }

    /// Write a full array of block pointers back into an indirect block.
    fn write_indirect_full(&self, block: u32, ptrs: &[u32]) -> Result<(), FsError> {
        let block_size = self.fs.block_size();
        let mut buf = vec![0u8; block_size];
        for (i, &p) in ptrs.iter().enumerate() {
            let off = i * 4;
            if off + 4 > block_size {
                break;
            }
            buf[off..off + 4].copy_from_slice(&p.to_le_bytes());
        }
        write_fs_block(&self.fs, block as usize, &buf)
    }

    // -----------------------------------------------------------------------
    // Block freeing / truncation
    // -----------------------------------------------------------------------

    /// Free every data and indirect block owned by this inode and reset the
    /// block pointers / `i_blocks` to zero.
    pub fn free_all_blocks(&self) -> Result<(), FsError> {
        let direct = { self.raw.lock().i_block };
        for &b in direct.iter().take(DIRECT_BLOCKS) {
            if b != 0 {
                self.fs.free_block(b)?;
            }
        }
        self.free_indirect(direct[12], 1)?;
        self.free_indirect(direct[13], 2)?;
        self.free_indirect(direct[14], 3)?;

        let mut raw = self.raw.lock();
        raw.i_block = [0; 15];
        raw.i_blocks = 0;
        Ok(())
    }

    /// Recursively free an indirect block tree (data blocks and metadata).
    fn free_indirect(&self, block: u32, level: u8) -> Result<(), FsError> {
        if block == 0 {
            return Ok(());
        }
        let ptrs = self.read_indirect_block(block)?;
        for &p in &ptrs {
            if p == 0 {
                continue;
            }
            if level == 1 {
                self.fs.free_block(p)?;
            } else {
                self.free_indirect(p, level - 1)?;
            }
        }
        self.fs.free_block(block)
    }

    /// Free the single data block backing `logical_block` (if any) and clear
    /// its pointer.  Indirect metadata is pruned separately.
    fn free_and_clear_logical(&self, logical_block: u32) -> Result<(), FsError> {
        let phys = self.resolve_block_id(logical_block)?;
        if phys != 0 {
            self.fs.free_block(phys)?;
        }

        let block_size = self.fs.block_size();
        let ptrs_per_block = (block_size / 4) as u32;
        let direct_limit = DIRECT_BLOCKS as u32;
        let indirect_limit = direct_limit + ptrs_per_block;
        let double_limit = indirect_limit + ptrs_per_block * ptrs_per_block;

        if logical_block < direct_limit {
            self.raw.lock().i_block[logical_block as usize] = 0;
        } else if logical_block < indirect_limit {
            let ib = { self.raw.lock().i_block[12] };
            if ib != 0 {
                self.write_indirect_entry(ib, logical_block - direct_limit, 0)?;
            }
        } else if logical_block < double_limit {
            let l1 = { self.raw.lock().i_block[13] };
            if l1 != 0 {
                let rem = logical_block - indirect_limit;
                let l2 = self.read_indirect_entry(l1, rem / ptrs_per_block)?;
                if l2 != 0 {
                    self.write_indirect_entry(l2, rem % ptrs_per_block, 0)?;
                }
            }
        } else {
            let l1 = { self.raw.lock().i_block[14] };
            if l1 != 0 {
                let rem = logical_block - double_limit;
                let p2 = ptrs_per_block * ptrs_per_block;
                let l2 = self.read_indirect_entry(l1, rem / p2)?;
                if l2 != 0 {
                    let l3 = self.read_indirect_entry(l2, (rem / ptrs_per_block) % ptrs_per_block)?;
                    if l3 != 0 {
                        self.write_indirect_entry(l3, rem % ptrs_per_block, 0)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Free indirect metadata blocks that no longer reference any data block,
    /// starting from the three top-level indirect pointers.
    fn prune_empty_indirects(&self) -> Result<(), FsError> {
        for slot in [12usize, 13, 14] {
            let level = (slot - 11) as u8; // 12→1, 13→2, 14→3
            let block = { self.raw.lock().i_block[slot] };
            if block != 0 && self.prune_indirect(block, level)? {
                self.fs.free_block(block)?;
                self.raw.lock().i_block[slot] = 0;
            }
        }
        Ok(())
    }

    /// Prune empty sub-blocks of an indirect tree; returns `true` if `block`
    /// ends up entirely empty (and may be freed by the caller).
    fn prune_indirect(&self, block: u32, level: u8) -> Result<bool, FsError> {
        if block == 0 {
            return Ok(true);
        }
        let mut ptrs = self.read_indirect_block(block)?;
        if level > 1 {
            let mut changed = false;
            for p in ptrs.iter_mut() {
                if *p != 0 && self.prune_indirect(*p, level - 1)? {
                    self.fs.free_block(*p)?;
                    *p = 0;
                    changed = true;
                }
            }
            if changed {
                self.write_indirect_full(block, &ptrs)?;
            }
        }
        Ok(ptrs.iter().all(|&p| p == 0))
    }

    /// Shared implementation of file truncation / extension.
    fn truncate_to(&self, new_size: u64) -> Result<(), FsError> {
        let cur = self.size();
        let block_size = self.fs.block_size() as u64;

        if new_size == 0 {
            self.free_all_blocks()?;
            self.set_size(0);
        } else if new_size >= cur {
            // Grow: leave holes; blocks are allocated lazily on write.
            self.set_size(new_size);
        } else {
            let keep = new_size.div_ceil(block_size) as u32;
            let old = cur.div_ceil(block_size) as u32;
            for lb in keep..old {
                self.free_and_clear_logical(lb)?;
            }
            self.prune_empty_indirects()?;
            self.set_size(new_size);
            self.recompute_i_blocks()?;
        }

        {
            let mut raw = self.raw.lock();
            raw.i_mtime = 0;
        }
        self.write_inode_to_disk()
    }

    // -----------------------------------------------------------------------
    // Symbolic links
    // -----------------------------------------------------------------------

    /// Read the target path of a symbolic link.
    fn read_symlink(&self) -> Result<String, FsError> {
        let (mode, size, i_blocks) = {
            let raw = self.raw.lock();
            (raw.i_mode, raw.i_size as usize, raw.i_blocks)
        };
        if mode & EXT2_S_IFMT != EXT2_S_IFLNK {
            return Err(FsError::InvalidInput);
        }

        let mut bytes = vec![0u8; size];
        // A "fast" symlink stores its target inline in the i_block array and
        // therefore allocates no data blocks (i_blocks == 0), which is the
        // authoritative test Linux uses (a slow link of ≤60 bytes is possible).
        if i_blocks == 0 && size <= FAST_SYMLINK_MAX {
            // Fast symlink: target stored inline in the i_block array.
            let raw = self.raw.lock();
            // SAFETY: i_block is [u32; 15] = 60 contiguous bytes.  We take a
            // raw pointer to the (possibly unaligned) packed field and read
            // the first `size` bytes; a `u8` slice has alignment 1.
            let base = core::ptr::addr_of!(raw.i_block) as *const u8;
            let inline = unsafe { core::slice::from_raw_parts(base, FAST_SYMLINK_MAX) };
            bytes.copy_from_slice(&inline[..size]);
        } else {
            self.read_at(0, &mut bytes)?;
        }
        String::from_utf8(bytes).map_err(|_| FsError::InvalidInput)
    }

    /// Write `target` into a freshly-created symlink inode.
    fn write_symlink_target(&self, target: &str) -> Result<(), FsError> {
        let bytes = target.as_bytes();
        if bytes.len() <= FAST_SYMLINK_MAX {
            {
                let mut raw = self.raw.lock();
                // SAFETY: i_block is 60 contiguous bytes.  Take a raw pointer
                // to the packed field (a `u8` slice has alignment 1) and write
                // the target inline, zeroing the remainder.
                let base = core::ptr::addr_of_mut!(raw.i_block) as *mut u8;
                let inline = unsafe { core::slice::from_raw_parts_mut(base, FAST_SYMLINK_MAX) };
                inline.fill(0);
                inline[..bytes.len()].copy_from_slice(bytes);
                raw.i_size = bytes.len() as u32;
            }
            self.write_inode_to_disk()
        } else {
            self.write_at(0, bytes)?;
            Ok(())
        }
    }

    /// Release this inode's extended-attribute block (`i_file_acl`), if any,
    /// honouring the shared-block reference count.
    ///
    /// ext2 xattr blocks may be shared by several inodes via an `h_refcount`
    /// field; the block is only freed when the last reference drops.  This
    /// keeps deletions of Linux-created files with xattrs consistent for
    /// `fsck`.
    pub(super) fn release_xattr_block(&self) -> Result<(), FsError> {
        let acl = { self.raw.lock().i_file_acl };
        if acl == 0 {
            return Ok(());
        }

        let block_size = self.fs.block_size();
        let mut buf = vec![0u8; block_size];
        read_fs_block(&self.fs, acl as usize, &mut buf)?;

        let magic = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        if magic == EXT2_XATTR_MAGIC {
            let refcount = u32::from_le_bytes([
                buf[XATTR_REFCOUNT_OFFSET],
                buf[XATTR_REFCOUNT_OFFSET + 1],
                buf[XATTR_REFCOUNT_OFFSET + 2],
                buf[XATTR_REFCOUNT_OFFSET + 3],
            ]);
            if refcount > 1 {
                // Still shared: just decrement the reference count.
                let dec = refcount - 1;
                buf[XATTR_REFCOUNT_OFFSET..XATTR_REFCOUNT_OFFSET + 4]
                    .copy_from_slice(&dec.to_le_bytes());
                write_fs_block(&self.fs, acl as usize, &buf)?;
                self.raw.lock().i_file_acl = 0;
                return Ok(());
            }
        }
        // Not shared (or not a recognised header): free the block outright.
        self.fs.free_block(acl)?;
        self.raw.lock().i_file_acl = 0;
        Ok(())
    }

    /// Return `true` if the directory `ancestor_ino` is equal to, or an
    /// ancestor of, the directory `start_ino` — walking up via `..` to the
    /// filesystem root.  Used to reject directory renames that would create a
    /// cycle.
    fn dir_is_ancestor_or_self(
        &self,
        ancestor_ino: u32,
        start_ino: u32,
    ) -> Result<bool, FsError> {
        let mut cur = start_ino;
        // The tree depth is bounded by the number of inodes; cap iterations to
        // avoid spinning forever on a pre-existing corrupt cycle.
        for _ in 0..=self.fs.super_block().inodes_count {
            if cur == ancestor_ino {
                return Ok(true);
            }
            if cur == EXT2_ROOT_INO {
                return Ok(false);
            }
            let dir = self.fs.get_inode(cur)?;
            let parent = super::dir::lookup(&dir, "..")?;
            if parent == cur {
                return Ok(false);
            }
            cur = parent;
        }
        // Depth limit hit — treat as a cycle to stay safe.
        Ok(true)
    }

    /// Write a freshly-serialized xattr block image, choosing the target
    /// block based on the current state:
    ///
    /// * no block yet (`acl == 0`): allocate one and grow `i_blocks`;
    /// * shared block (`refcount > 1`): copy-on-write to a new block and
    ///   decrement the old reference count;
    /// * privately-owned block: overwrite in place.
    ///
    /// Updates `i_file_acl` accordingly.
    fn store_xattr_block(
        &self,
        acl: u32,
        shared: bool,
        new_buf: &[u8],
    ) -> Result<(), FsError> {
        if acl == 0 {
            let block = self.fs.alloc_block_goal(self.block_group())?;
            write_fs_block(&self.fs, block as usize, new_buf)?;
            let mut raw = self.raw.lock();
            raw.i_file_acl = block;
            let sectors = self.sectors_per_block();
            raw.i_blocks = raw.i_blocks.saturating_add(sectors);
        } else if shared {
            // Copy-on-write: never mutate a block another inode still shares.
            let block = self.fs.alloc_block_goal(self.block_group())?;
            write_fs_block(&self.fs, block as usize, new_buf)?;
            self.decrement_xattr_refcount(acl)?;
            self.raw.lock().i_file_acl = block;
            // i_blocks is unchanged: we swapped one owned block for another.
        } else {
            write_fs_block(&self.fs, acl as usize, new_buf)?;
        }
        Ok(())
    }

    /// Decrement the reference count of a shared xattr block (it stays alive
    /// because `refcount` was greater than one).
    fn decrement_xattr_refcount(&self, block: u32) -> Result<(), FsError> {
        let mut buf = vec![0u8; self.fs.block_size()];
        read_fs_block(&self.fs, block as usize, &mut buf)?;
        let rc = xattr::refcount(&buf);
        xattr::set_refcount(&mut buf, rc.saturating_sub(1));
        write_fs_block(&self.fs, block as usize, &buf)
    }

    /// If this directory is htree-indexed (`EXT2_INDEX_FL`), clear the flag so
    /// it can be safely modified as a plain linear directory.
    ///
    /// The on-disk htree layout keeps all real entries in leaf blocks that a
    /// linear scan already traverses, and the index root lives inside an
    /// entry marked unused, so dropping the flag yields a valid linear
    /// directory that both Koros and Linux read correctly.
    pub(super) fn demote_from_htree(&self) -> Result<(), FsError> {
        let indexed = { self.raw.lock().i_flags & EXT2_INDEX_FL != 0 };
        if indexed {
            self.raw.lock().i_flags &= !EXT2_INDEX_FL;
            self.write_inode_to_disk()?;
        }
        Ok(())
    }

    /// Shared implementation for `create` / `mknod` / `symlink`: allocate an
    /// inode, initialise it, and add a directory entry in `self`.
    ///
    /// For block/char devices, `rdev` is written into `i_block[0]`.
    fn create_child(
        &self,
        name: &str,
        file_type: FileType,
        mode: u32,
        rdev: u32,
    ) -> Result<Arc<dyn INode>, FsError> {
        if super::dir::lookup(self, name).is_ok() {
            return Err(FsError::AlreadyExists);
        }

        let new_inode_num = self.fs.alloc_inode()?;
        let ext2_mode = file_type_to_mode(file_type, mode);
        if let Err(e) = self.fs.init_inode(new_inode_num, ext2_mode) {
            let _ = self.fs.free_inode(new_inode_num);
            return Err(e);
        }

        let dir_ft = file_type_to_dir_byte(file_type);
        if let Err(e) = super::dir::add_dir_entry(self, name, new_inode_num, dir_ft) {
            let _ = self.fs.free_inode(new_inode_num);
            return Err(e);
        }

        let new_inode = self.fs.get_inode(new_inode_num)?;

        // Device files store their device number in the block-pointer array,
        // using the same old/new encoding as Linux so fsck and Linux read it
        // back correctly.
        if matches!(file_type, FileType::BlockDevice | FileType::CharDevice) {
            let (b0, b1) = encode_dev(rdev);
            {
                let mut raw = new_inode.raw();
                raw.i_block[0] = b0;
                raw.i_block[1] = b1;
            }
            new_inode.write_inode_to_disk()?;
        }

        Ok(new_inode as Arc<dyn INode>)
    }
}

/// Encode a device number into the `(i_block[0], i_block[1])` pair the way
/// Linux ext2 does.
///
/// `rdev` is a kernel `dev_t`: `major = rdev >> 20`, `minor = rdev & 0xFFFFF`.
/// Small numbers use the legacy 16-bit encoding in `i_block[0]`; larger ones
/// use the 32-bit encoding in `i_block[1]` (with `i_block[0] == 0`).
fn encode_dev(rdev: u32) -> (u32, u32) {
    let major = rdev >> 20;
    let minor = rdev & 0x000F_FFFF;
    if major < 256 && minor < 256 {
        // old_valid_dev / old_encode_dev
        ((major << 8) | minor, 0)
    } else {
        // new_encode_dev
        let encoded = (minor & 0xff) | (major << 8) | ((minor & !0xff) << 12);
        (0, encoded)
    }
}

// ---------------------------------------------------------------------------
// INode trait implementation
// ---------------------------------------------------------------------------

impl INode for Ext2INode {
    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
        let file_size = self.size() as usize;

        if offset >= file_size || buf.is_empty() {
            return Ok(0);
        }

        let block_size = self.fs.block_size();
        let bytes_to_read = core::cmp::min(buf.len(), file_size - offset);
        let mut bytes_read = 0;

        while bytes_read < bytes_to_read {
            let file_offset = offset + bytes_read;
            let logical_block = file_offset / block_size;
            let block_offset = file_offset % block_size;
            let chunk = core::cmp::min(bytes_to_read - bytes_read, block_size - block_offset);

            let phys_block = self.resolve_block_id(logical_block as u32)?;
            if phys_block == 0 {
                // Sparse block — fill with zeros.
                for b in &mut buf[bytes_read..bytes_read + chunk] {
                    *b = 0;
                }
            } else {
                let mut block_buf = vec![0u8; block_size];
                read_fs_block(&self.fs, phys_block as usize, &mut block_buf)?;
                buf[bytes_read..bytes_read + chunk]
                    .copy_from_slice(&block_buf[block_offset..block_offset + chunk]);
            }
            bytes_read += chunk;
        }

        Ok(bytes_read)
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize, FsError> {
        if buf.is_empty() {
            return Ok(0);
        }

        let block_size = self.fs.block_size();
        let end_offset = offset + buf.len();
        let mut bytes_written = 0usize;
        // Filesystem blocks newly allocated during this write, tracked so
        // `i_blocks` can be updated incrementally rather than rescanned.
        let mut new_fs_blocks = 0u32;

        while bytes_written < buf.len() {
            let file_offset = offset + bytes_written;
            let logical_block = file_offset / block_size;
            let block_offset = file_offset % block_size;
            let chunk = core::cmp::min(buf.len() - bytes_written, block_size - block_offset);

            // Get or allocate the physical block for this logical position.
            let (phys_block, allocated) = self.allocate_block_for_offset(logical_block as u32)?;
            new_fs_blocks += allocated;

            if chunk == block_size && block_offset == 0 {
                // Full-block write — write directly from the caller's buffer.
                write_fs_block(
                    &self.fs,
                    phys_block as usize,
                    &buf[bytes_written..bytes_written + chunk],
                )?;
            } else {
                // Partial-block write — read-modify-write.
                let mut block_buf = vec![0u8; block_size];
                read_fs_block(&self.fs, phys_block as usize, &mut block_buf)?;
                block_buf[block_offset..block_offset + chunk]
                    .copy_from_slice(&buf[bytes_written..bytes_written + chunk]);
                write_fs_block(&self.fs, phys_block as usize, &block_buf)?;
            }

            bytes_written += chunk;
        }

        // Grow the logical size if we wrote past the current EOF.
        if end_offset as u64 > self.size() {
            self.set_size(end_offset as u64);
        }
        {
            let mut raw = self.raw.lock();
            // TODO: use real wall-clock time once the timer subsystem exists.
            raw.i_mtime = 0;
            // Add only the blocks allocated by this write to i_blocks; the
            // starting value already accounts for pre-existing data, indirect
            // and xattr blocks.
            if new_fs_blocks > 0 {
                let sectors = new_fs_blocks * self.sectors_per_block();
                raw.i_blocks = raw.i_blocks.saturating_add(sectors);
            }
        }

        // Persist inode to disk.
        self.write_inode_to_disk()?;

        Ok(bytes_written)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn INode>, FsError> {
        let inode_num = super::dir::lookup(self, name)?;
        Ok(self.fs.get_inode(inode_num)? as Arc<dyn INode>)
    }

    fn create(
        &self,
        name: &str,
        file_type: FileType,
        mode: u32,
    ) -> Result<Arc<dyn INode>, FsError> {
        // Directories must go through `mkdir` (which sets up "." / "..").
        if file_type == FileType::Directory {
            return Err(FsError::InvalidInput);
        }
        self.create_child(name, file_type, mode, 0)
    }

    fn unlink(&self, name: &str) -> Result<(), FsError> {
        let child_inode_num = super::dir::lookup(self, name)?;

        let child_inode = self.fs.get_inode(child_inode_num)?;
        {
            let raw = child_inode.raw();
            if raw.i_mode & EXT2_S_IFMT == EXT2_S_IFDIR {
                return Err(FsError::IsDirectory);
            }
        }

        super::dir::remove_dir_entry(self, name)?;

        let should_free = {
            let mut raw = child_inode.raw();
            raw.i_links_count = raw.i_links_count.saturating_sub(1);
            raw.i_links_count == 0
        };

        if should_free {
            // Release the file's data blocks and any extended-attribute block
            // before returning the inode to the free pool, and record the
            // deletion time.
            child_inode.free_all_blocks()?;
            child_inode.release_xattr_block()?;
            {
                let mut raw = child_inode.raw();
                raw.i_dtime = 0; // TODO: real wall-clock time once available.
            }
            child_inode.write_inode_to_disk()?;
            self.fs.free_inode(child_inode_num)?;
        } else {
            child_inode.write_inode_to_disk()?;
        }

        Ok(())
    }

    fn mkdir(&self, name: &str, mode: u32) -> Result<Arc<dyn INode>, FsError> {
        if super::dir::lookup(self, name).is_ok() {
            return Err(FsError::AlreadyExists);
        }

        let new_inode_num = self.fs.alloc_inode()?;
        let ext2_mode = EXT2_S_IFDIR | (mode as u16 & 0x0FFF);
        if let Err(e) = self.fs.init_inode(new_inode_num, ext2_mode) {
            let _ = self.fs.free_inode(new_inode_num);
            return Err(e);
        }

        if let Err(e) = super::dir::add_dir_entry(self, name, new_inode_num, super::dir::TYPE_DIRECTORY) {
            let _ = self.fs.free_inode(new_inode_num);
            return Err(e);
        }

        let new_inode = self.fs.get_inode(new_inode_num)?;

        super::dir::add_dir_entry(&new_inode, ".", new_inode_num, super::dir::TYPE_DIRECTORY)?;
        super::dir::add_dir_entry(&new_inode, "..", self.inode_num, super::dir::TYPE_DIRECTORY)?;

        {
            let mut raw = new_inode.raw();
            raw.i_links_count = 2;
        }
        new_inode.write_inode_to_disk()?;

        {
            let mut raw = self.raw();
            raw.i_links_count += 1;
        }
        self.write_inode_to_disk()?;

        // Account for the new directory in its block group.
        self.fs.adjust_used_dirs(new_inode_num, 1);

        Ok(new_inode as Arc<dyn INode>)
    }

    fn rmdir(&self, name: &str) -> Result<(), FsError> {
        if name == "." || name == ".." {
            return Err(FsError::InvalidInput);
        }
        let child_inode_num = super::dir::lookup(self, name)?;

        let child_inode = self.fs.get_inode(child_inode_num)?;
        {
            let raw = child_inode.raw();
            if raw.i_mode & EXT2_S_IFMT != EXT2_S_IFDIR {
                return Err(FsError::NotDirectory);
            }
        }

        let entries = super::dir::list(&child_inode)?;
        let has_children = entries
            .iter()
            .any(|(n, _)| n.as_str() != "." && n.as_str() != "..");
        if has_children {
            return Err(FsError::NotEmpty);
        }

        super::dir::remove_dir_entry(self, name)?;

        // The parent loses the child's ".." backlink.
        {
            let mut raw = self.raw();
            raw.i_links_count = raw.i_links_count.saturating_sub(1);
        }
        self.write_inode_to_disk()?;

        // Release the directory's own data blocks and xattr block, then the
        // inode.
        child_inode.free_all_blocks()?;
        child_inode.release_xattr_block()?;
        {
            let mut raw = child_inode.raw();
            raw.i_links_count = 0;
            raw.i_dtime = 0;
        }
        child_inode.write_inode_to_disk()?;
        self.fs.adjust_used_dirs(child_inode_num, -1);
        self.fs.free_inode(child_inode_num)?;

        Ok(())
    }

    fn getattr(&self) -> Result<Metadata, FsError> {
        let size = self.size() as usize;
        let raw = self.raw.lock();
        let mode = raw.i_mode;
        let file_type = mode_to_file_type(mode);

        Ok(Metadata {
            size,
            mode: mode as u32,
            uid: raw.i_uid as u32,
            gid: raw.i_gid as u32,
            atime: raw.i_atime as u64,
            mtime: raw.i_mtime as u64,
            ctime: raw.i_ctime as u64,
            file_type,
        })
    }

    fn setattr(&self, metadata: &Metadata) -> Result<(), FsError> {
        {
            let mut raw = self.raw.lock();
            raw.i_mode = metadata.mode as u16;
            raw.i_uid = metadata.uid as u16;
            raw.i_gid = metadata.gid as u16;
            raw.i_atime = metadata.atime as u32;
            raw.i_mtime = metadata.mtime as u32;
            raw.i_ctime = metadata.ctime as u32;
            // Only update size for regular files.
            if (raw.i_mode & EXT2_S_IFMT) == EXT2_S_IFREG {
                raw.i_size = metadata.size as u32;
                let block_size = self.fs.block_size();
                let total_fs_blocks = (metadata.size).div_ceil(block_size);
                raw.i_blocks = (total_fs_blocks * (block_size / 512)) as u32;
            }
        }
        self.write_inode_to_disk()
    }

    fn list(&self) -> Result<Vec<(String, u32)>, FsError> {
        super::dir::list(self)
    }

    fn ino(&self) -> u32 {
        self.inode_num
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn readlink(&self) -> Result<String, FsError> {
        self.read_symlink()
    }

    fn symlink(&self, name: &str, target: &str) -> Result<Arc<dyn INode>, FsError> {
        let node = self.create_child(name, FileType::Symlink, 0o777, 0)?;
        // Downcast to write the target inline / into a data block.
        let ext2: &Ext2INode = node
            .as_any()
            .downcast_ref::<Ext2INode>()
            .ok_or(FsError::IoError)?;
        ext2.write_symlink_target(target)?;
        Ok(node)
    }

    fn link(&self, name: &str, target: &Arc<dyn INode>) -> Result<(), FsError> {
        if super::dir::lookup(self, name).is_ok() {
            return Err(FsError::AlreadyExists);
        }
        let target_ext2 = target
            .as_any()
            .downcast_ref::<Ext2INode>()
            .ok_or(FsError::CrossDevice)?;

        // Hard links to directories are not permitted.
        let (target_ino, dir_ft) = {
            let raw = target_ext2.raw();
            if raw.i_mode & EXT2_S_IFMT == EXT2_S_IFDIR {
                return Err(FsError::IsDirectory);
            }
            (target_ext2.inode_num, dir_byte_for_mode(raw.i_mode))
        };

        super::dir::add_dir_entry(self, name, target_ino, dir_ft)?;
        {
            let mut raw = target_ext2.raw();
            raw.i_links_count = raw.i_links_count.saturating_add(1);
        }
        target_ext2.write_inode_to_disk()
    }

    fn rename(
        &self,
        old_name: &str,
        new_parent: &Arc<dyn INode>,
        new_name: &str,
    ) -> Result<(), FsError> {
        let new_dir = new_parent
            .as_any()
            .downcast_ref::<Ext2INode>()
            .ok_or(FsError::CrossDevice)?;

        let child_ino = super::dir::lookup(self, old_name)?;
        let child = self.fs.get_inode(child_ino)?;
        let dir_ft = dir_byte_for_mode(child.raw().i_mode);
        let child_is_dir = child.raw().i_mode & EXT2_S_IFMT == EXT2_S_IFDIR;

        // Moving a directory into itself or into one of its own descendants
        // would create a detached cycle — reject it (POSIX EINVAL).
        if child_is_dir && self.dir_is_ancestor_or_self(child_ino, new_dir.inode_num)? {
            return Err(FsError::InvalidInput);
        }

        // If the destination already exists, remove it first (must be the
        // same type and, for directories, empty).
        if let Ok(existing_ino) = super::dir::lookup(new_dir, new_name) {
            if existing_ino == child_ino {
                return Ok(());
            }
            let existing = self.fs.get_inode(existing_ino)?;
            let existing_is_dir = existing.raw().i_mode & EXT2_S_IFMT == EXT2_S_IFDIR;
            if existing_is_dir {
                let entries = super::dir::list(&existing)?;
                if entries
                    .iter()
                    .any(|(n, _)| n.as_str() != "." && n.as_str() != "..")
                {
                    return Err(FsError::NotEmpty);
                }
                new_dir.rmdir(new_name)?;
            } else {
                new_dir.unlink(new_name)?;
            }
        }

        // Add the entry under the new name, then drop the old one.
        super::dir::add_dir_entry(new_dir, new_name, child_ino, dir_ft)?;
        super::dir::remove_dir_entry(self, old_name)?;

        // Moving a directory across parents updates ".." and link counts.
        if child_is_dir && !core::ptr::eq(self as *const _, new_dir as *const _) {
            super::dir::remove_dir_entry(&child, "..")?;
            super::dir::add_dir_entry(&child, "..", new_dir.inode_num, super::dir::TYPE_DIRECTORY)?;
            {
                let mut raw = self.raw();
                raw.i_links_count = raw.i_links_count.saturating_sub(1);
            }
            self.write_inode_to_disk()?;
            {
                let mut raw = new_dir.raw();
                raw.i_links_count = raw.i_links_count.saturating_add(1);
            }
            new_dir.write_inode_to_disk()?;
            self.fs.adjust_used_dirs(self.inode_num, -1);
            self.fs.adjust_used_dirs(new_dir.inode_num, 1);
        }
        Ok(())
    }

    fn truncate(&self, size: usize) -> Result<(), FsError> {
        {
            let raw = self.raw.lock();
            if raw.i_mode & EXT2_S_IFMT != EXT2_S_IFREG {
                return Err(FsError::InvalidInput);
            }
        }
        self.truncate_to(size as u64)
    }

    fn mknod(
        &self,
        name: &str,
        file_type: FileType,
        mode: u32,
        rdev: u32,
    ) -> Result<Arc<dyn INode>, FsError> {
        match file_type {
            FileType::BlockDevice
            | FileType::CharDevice
            | FileType::Fifo
            | FileType::Socket => self.create_child(name, file_type, mode, rdev),
            _ => Err(FsError::InvalidInput),
        }
    }

    fn getxattr(&self, name: &str) -> Result<Vec<u8>, FsError> {
        let (index, suffix) = xattr::split_name(name).ok_or(FsError::InvalidInput)?;
        let acl = { self.raw.lock().i_file_acl };
        if acl == 0 {
            return Err(FsError::NotFound);
        }
        let mut buf = vec![0u8; self.fs.block_size()];
        read_fs_block(&self.fs, acl as usize, &mut buf)?;
        for a in xattr::parse_block(&buf)? {
            if a.index == index && a.name == suffix.as_bytes() {
                return Ok(a.value);
            }
        }
        Err(FsError::NotFound)
    }

    fn listxattr(&self) -> Result<Vec<String>, FsError> {
        let acl = { self.raw.lock().i_file_acl };
        if acl == 0 {
            return Ok(Vec::new());
        }
        let mut buf = vec![0u8; self.fs.block_size()];
        read_fs_block(&self.fs, acl as usize, &mut buf)?;
        let mut names = Vec::new();
        for a in xattr::parse_block(&buf)? {
            if let Some(full) = xattr::full_name(a.index, &a.name) {
                names.push(full);
            }
        }
        Ok(names)
    }

    fn setxattr(&self, name: &str, value: &[u8]) -> Result<(), FsError> {
        let (index, suffix) = xattr::split_name(name).ok_or(FsError::InvalidInput)?;
        let block_size = self.fs.block_size();

        // Load existing attributes (and note whether the block is shared).
        let acl = { self.raw.lock().i_file_acl };
        let (mut attrs, shared) = if acl != 0 {
            let mut buf = vec![0u8; block_size];
            read_fs_block(&self.fs, acl as usize, &mut buf)?;
            (xattr::parse_block(&buf)?, xattr::refcount(&buf) > 1)
        } else {
            (Vec::new(), false)
        };

        // Upsert the attribute.
        attrs.retain(|a| !(a.index == index && a.name == suffix.as_bytes()));
        attrs.push(xattr::Attr {
            index,
            name: suffix.as_bytes().to_vec(),
            value: value.to_vec(),
        });

        let new_buf = xattr::serialize_block(&mut attrs, block_size)?;
        self.store_xattr_block(acl, shared, &new_buf)?;
        self.write_inode_to_disk()
    }

    fn removexattr(&self, name: &str) -> Result<(), FsError> {
        let (index, suffix) = xattr::split_name(name).ok_or(FsError::InvalidInput)?;
        let block_size = self.fs.block_size();

        let acl = { self.raw.lock().i_file_acl };
        if acl == 0 {
            return Err(FsError::NotFound);
        }
        let mut buf = vec![0u8; block_size];
        read_fs_block(&self.fs, acl as usize, &mut buf)?;
        let shared = xattr::refcount(&buf) > 1;
        let mut attrs = xattr::parse_block(&buf)?;

        let before = attrs.len();
        attrs.retain(|a| !(a.index == index && a.name == suffix.as_bytes()));
        if attrs.len() == before {
            return Err(FsError::NotFound);
        }

        if attrs.is_empty() {
            // No attributes remain: drop our reference to the block.
            if shared {
                self.decrement_xattr_refcount(acl)?;
            } else {
                self.fs.free_block(acl)?;
            }
            {
                let mut raw = self.raw.lock();
                raw.i_file_acl = 0;
                let sectors = self.sectors_per_block();
                raw.i_blocks = raw.i_blocks.saturating_sub(sectors);
            }
        } else {
            let new_buf = xattr::serialize_block(&mut attrs, block_size)?;
            self.store_xattr_block(acl, shared, &new_buf)?;
        }
        self.write_inode_to_disk()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Reinterpret a byte slice as a reference to `T`.
///
/// # Safety (logical)
///
/// `T` must be `repr(C, packed)` (alignment 1) and the slice must be large
/// enough.  We copy the value out of the potentially-unaligned pointer via
/// `core::ptr::read_unaligned`.
fn bytemuck_ref<T: Copy>(bytes: &[u8]) -> &T {
    assert!(bytes.len() >= core::mem::size_of::<T>());
    // SAFETY: `T` is `repr(C, packed)` (alignment 1) and the slice is large
    // enough.  The pointer cast is valid for packed types.
    unsafe { &*(bytes.as_ptr().cast::<T>()) }
}

fn file_type_to_mode(file_type: FileType, mode: u32) -> u16 {
    let type_bits: u16 = match file_type {
        FileType::Regular => EXT2_S_IFREG,
        FileType::Directory => EXT2_S_IFDIR,
        FileType::Symlink => EXT2_S_IFLNK,
        FileType::BlockDevice => EXT2_S_IFBLK,
        FileType::CharDevice => EXT2_S_IFCHR,
        FileType::Fifo => EXT2_S_IFIFO,
        FileType::Socket => EXT2_S_IFSOCK,
    };
    type_bits | (mode as u16 & 0x0FFF)
}

fn file_type_to_dir_byte(file_type: FileType) -> u8 {
    match file_type {
        FileType::Regular => super::dir::TYPE_REGULAR,
        FileType::Directory => super::dir::TYPE_DIRECTORY,
        FileType::Symlink => super::dir::TYPE_SYMLINK,
        FileType::BlockDevice => super::dir::TYPE_BLOCK_DEVICE,
        FileType::CharDevice => super::dir::TYPE_CHAR_DEVICE,
        FileType::Fifo => super::dir::TYPE_FIFO,
        FileType::Socket => super::dir::TYPE_SOCKET,
    }
}

/// Map the type bits of an `i_mode` value to the VFS [`FileType`].
fn mode_to_file_type(mode: u16) -> FileType {
    match mode & EXT2_S_IFMT {
        EXT2_S_IFREG => FileType::Regular,
        EXT2_S_IFDIR => FileType::Directory,
        EXT2_S_IFLNK => FileType::Symlink,
        EXT2_S_IFBLK => FileType::BlockDevice,
        EXT2_S_IFCHR => FileType::CharDevice,
        EXT2_S_IFIFO => FileType::Fifo,
        EXT2_S_IFSOCK => FileType::Socket,
        _ => FileType::Regular,
    }
}

/// The directory-entry file-type byte corresponding to an `i_mode` value.
fn dir_byte_for_mode(mode: u16) -> u8 {
    file_type_to_dir_byte(mode_to_file_type(mode))
}

/// Read a filesystem-level block from the device backing `fs`.
///
/// Converts the filesystem block number to device block(s) and reads the
/// entire filesystem block into `buf` (which must be exactly `fs.block_size()`
/// bytes).
fn read_fs_block(fs: &Ext2Fs, fs_block: usize, buf: &mut [u8]) -> Result<(), FsError> {
    fs.read_fs_block(fs_block, buf)
}

/// Write `buf` (exactly `fs.block_size()` bytes) to a filesystem-level block
/// on the device backing `fs`.
fn write_fs_block(fs: &Ext2Fs, fs_block: usize, buf: &[u8]) -> Result<(), FsError> {
    fs.write_fs_block(fs_block, buf)
}
