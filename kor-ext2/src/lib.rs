#![no_std]
//! ext2 filesystem implementation.
//!
//! Provides read-only ext2 support by parsing the on-disk superblock
//! and block group descriptor table.

pub mod bitmap;
pub mod block_group;
pub mod dir;
pub mod inode;
pub mod super_block;
pub mod xattr;

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use spin::Mutex;

use kor::BlockDevice;
use kor_fs::block_cache::BlockCache;
use kor::{FsError, FsInfo, INode, SuperBlock as SuperBlockTrait};

use block_group::BlockGroupDescriptor;
use inode::Ext2INode;
use super_block::{RawSuperBlock, SuperBlock, EXT2_VALID_FS};

/// Byte offset of the ext2 superblock from the start of the device.
const SUPERBLOCK_OFFSET: usize = 1024;

/// The root directory is always inode number 2 in ext2.
pub const EXT2_ROOT_INO: u32 = 2;

/// Number of device blocks the block cache may hold (≈ 512 KiB at 512-byte
/// blocks).
const BLOCK_CACHE_CAPACITY: usize = 1024;

/// Once the inode cache reaches this many entries, a `get_inode` miss sweeps
/// out entries whose inode is no longer live (dead `Weak`s).
const INODE_CACHE_PRUNE_THRESHOLD: usize = 256;

// ---------------------------------------------------------------------------
// Ext2Fs — the mounted ext2 filesystem instance
// ---------------------------------------------------------------------------

/// An ext2 filesystem instance.
///
/// Mutable metadata (superblock, block group descriptors) is wrapped in
/// [`spin::Mutex`] so that allocation methods can update bookkeeping
/// through shared `&self` references.
pub struct Ext2Fs {
    /// The underlying block device.
    device: Arc<dyn BlockDevice>,
    /// Write-back LRU cache sitting in front of `device`.
    cache: Mutex<BlockCache>,
    super_block: Mutex<SuperBlock>,
    block_groups: Mutex<Vec<BlockGroupDescriptor>>,
    /// Filesystem block size in bytes (derived from `super_block.log_block_size`).
    block_size: usize,
    /// When `true`, all mutating operations are rejected (the filesystem has
    /// unsupported ro_compat features).
    read_only: bool,
    /// Weak self-reference so `&self` methods can hand an `Arc<Ext2Fs>` to
    /// newly-constructed inodes.
    self_ref: Mutex<Weak<Ext2Fs>>,
    /// Cache of live inodes, keyed by inode number.  Stored as `Weak` so an
    /// inode is dropped once no `Arc` holder remains, while guaranteeing at
    /// most one live in-memory copy per inode number.
    inode_cache: Mutex<BTreeMap<u32, Weak<Ext2INode>>>,
}

impl Ext2Fs {
    /// Open an ext2 filesystem on `device`.
    ///
    /// Reads and validates the superblock, then loads the block group
    /// descriptor table.  Returns [`FsError::InvalidInput`] if the
    /// superblock magic number is wrong or the device is too small.
    pub fn open(device: Arc<dyn BlockDevice>) -> Result<Arc<Self>, FsError> {
        let dev_block_size = device.block_size();

        // ------------------------------------------------------------------
        // 1. Read the superblock (always at byte offset 1024, length 1024).
        // ------------------------------------------------------------------
        let sb_block = SUPERBLOCK_OFFSET / dev_block_size;
        let sb_offset_in_block = SUPERBLOCK_OFFSET % dev_block_size;

        let sb_bytes_needed = 1024;
        let blocks_needed = (sb_bytes_needed + sb_offset_in_block).div_ceil(dev_block_size);
        let mut sb_buf = vec![0u8; blocks_needed * dev_block_size];
        for i in 0..blocks_needed {
            let start = i * dev_block_size;
            let end = start + dev_block_size;
            device
                .read_block(sb_block + i, &mut sb_buf[start..end])
                .map_err(|_| FsError::IoError)?;
        }

        // Interpret the buffer as a raw superblock.
        let raw_sb: &RawSuperBlock = bytemuck_or_ref(&sb_buf[sb_offset_in_block..]);
        let super_block = SuperBlock::from_raw(raw_sb)?;
        let block_size = super_block.block_size();

        // The filesystem block must be a whole multiple of the device block so
        // that fs-block ↔ device-block conversion is exact.
        if block_size < dev_block_size || block_size % dev_block_size != 0 {
            return Err(FsError::InvalidInput);
        }

        // ------------------------------------------------------------------
        // 2. Read the block group descriptor table.
        // ------------------------------------------------------------------
        // The descriptor table starts at the block immediately following the
        // superblock.  For 1024-byte fs blocks that is block 2; for larger
        // blocks it is block 1 (the superblock is within block 0/1 depending
        // on layout — ext2 always uses block 1 for the superblock when
        // block_size == 1024, and the descriptor table is the next block).
        //
        // The descriptor table block number depends on the filesystem block
        // size and the superblock location:
        //   - block_size == 1024: superblock is in block 1, descriptors in block 2
        //   - block_size > 1024:  superblock is in block 0, descriptors in block 1
        let bgdt_block = if block_size == 1024 { 2 } else { 1 };

        let bg_count = super_block.block_group_count() as usize;
        let bgdt_bytes = bg_count * core::mem::size_of::<BlockGroupDescriptor>();
        let bgdt_blocks = bgdt_bytes.div_ceil(block_size);
        let mut bgdt_buf = vec![0u8; bgdt_blocks * block_size];
        // Read the BGDT one filesystem block at a time.  Each filesystem
        // block maps to `block_size / dev_block_size` device blocks.
        let fs_per_dev = block_size / dev_block_size;
        for i in 0..bgdt_blocks {
            let fs_block = bgdt_block + i;
            let dev_start = fs_block * fs_per_dev;
            let buf_offset = i * block_size;
            for j in 0..fs_per_dev {
                let start = buf_offset + j * dev_block_size;
                let end = start + dev_block_size;
                device
                    .read_block(dev_start + j, &mut bgdt_buf[start..end])
                    .map_err(|_| FsError::IoError)?;
            }
        }

        let block_groups = parse_block_group_descriptors(&bgdt_buf, bg_count);

        // ------------------------------------------------------------------
        // 3. Reject filesystems with unsupported incompatible features; mount
        //    read-only if unknown ro_compat features are present.
        // ------------------------------------------------------------------
        if super_block.unsupported_incompat() != 0 {
            return Err(FsError::InvalidInput);
        }
        let read_only = super_block.unsupported_ro_compat() != 0;

        let cache = BlockCache::new(Arc::clone(&device), BLOCK_CACHE_CAPACITY);

        let fs = Arc::new(Self {
            device,
            cache: Mutex::new(cache),
            super_block: Mutex::new(super_block),
            block_groups: Mutex::new(block_groups),
            block_size,
            read_only,
            self_ref: Mutex::new(Weak::new()),
            inode_cache: Mutex::new(BTreeMap::new()),
        });
        *fs.self_ref.lock() = Arc::downgrade(&fs);
        Ok(fs)
    }

    /// The filesystem block size in bytes.
    pub fn block_size(&self) -> usize {
        self.block_size
    }

    /// A reference to the underlying block device.
    pub fn device(&self) -> &Arc<dyn BlockDevice> {
        &self.device
    }

    /// A reference to the parsed superblock.
    pub fn super_block(&self) -> spin::MutexGuard<'_, SuperBlock> {
        self.super_block.lock()
    }

    /// The block group descriptor table.
    pub fn block_groups(&self) -> spin::MutexGuard<'_, Vec<BlockGroupDescriptor>> {
        self.block_groups.lock()
    }

    /// `true` if the filesystem was mounted read-only.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// `true` if directory entries carry an explicit file-type byte.
    pub fn has_filetype(&self) -> bool {
        self.super_block.lock().has_filetype()
    }

    /// `true` if regular files may use the 64-bit `large_file` size encoding.
    pub fn has_large_file(&self) -> bool {
        self.super_block.lock().has_large_file()
    }

    // -----------------------------------------------------------------------
    // Filesystem-block I/O (routed through the block cache)
    // -----------------------------------------------------------------------

    /// Read filesystem block `fs_block` (exactly `block_size` bytes) into
    /// `buf` via the block cache.
    pub fn read_fs_block(&self, fs_block: usize, buf: &mut [u8]) -> Result<(), FsError> {
        let dev_bs = self.device.block_size();
        let per = self.block_size / dev_bs;
        let dev_start = fs_block * per;
        let mut cache = self.cache.lock();
        for i in 0..per {
            let start = i * dev_bs;
            cache
                .read_block_cached(dev_start + i, &mut buf[start..start + dev_bs])
                .map_err(|_| FsError::IoError)?;
        }
        Ok(())
    }

    /// Write `buf` (exactly `block_size` bytes) to filesystem block
    /// `fs_block` via the block cache.
    pub fn write_fs_block(&self, fs_block: usize, buf: &[u8]) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let dev_bs = self.device.block_size();
        let per = self.block_size / dev_bs;
        let dev_start = fs_block * per;
        let mut cache = self.cache.lock();
        for i in 0..per {
            let start = i * dev_bs;
            cache
                .write_block_cached(dev_start + i, &buf[start..start + dev_bs])
                .map_err(|_| FsError::IoError)?;
        }
        Ok(())
    }

    /// Read `count` contiguous filesystem blocks starting at `start_fs_block`
    /// in a single multi-block device request, bypassing the block cache.
    ///
    /// Dirty cached copies of the range are flushed first so the direct read
    /// observes the latest data.  `buf.len()` must equal `count * block_size`.
    pub fn read_fs_blocks(
        &self,
        start_fs_block: usize,
        count: usize,
        buf: &mut [u8],
    ) -> Result<(), FsError> {
        let per = self.block_size / self.device.block_size();
        let dev_start = start_fs_block * per;
        {
            let mut cache = self.cache.lock();
            cache
                .flush_range(dev_start, count * per)
                .map_err(|_| FsError::IoError)?;
        }
        self.device
            .read_blocks(dev_start, buf)
            .map_err(|_| FsError::IoError)
    }

    /// Write `count` contiguous filesystem blocks starting at `start_fs_block`
    /// in a single multi-block device request, bypassing the block cache.
    ///
    /// Any cached copies of the range are discarded first (the whole range is
    /// overwritten).  `buf.len()` must equal `count * block_size`.
    pub fn write_fs_blocks(
        &self,
        start_fs_block: usize,
        count: usize,
        buf: &[u8],
    ) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let per = self.block_size / self.device.block_size();
        let dev_start = start_fs_block * per;
        {
            let mut cache = self.cache.lock();
            cache.invalidate_range(dev_start, count * per);
        }
        self.device
            .write_blocks(dev_start, buf)
            .map_err(|_| FsError::IoError)
    }

    // -----------------------------------------------------------------------
    // Metadata persistence (superblock + block group descriptor table)
    // -----------------------------------------------------------------------

    /// Write the in-memory superblock counters back to the primary superblock
    /// and every backup copy.
    pub fn write_super_block(&self) -> Result<(), FsError> {
        if self.read_only {
            return Ok(());
        }
        let sb = self.super_block.lock();
        let block_size = self.block_size;

        // Primary superblock: at byte 1024. For 1 KiB blocks that is block 1
        // at offset 0; for larger blocks it is block 0 at offset 1024.
        let (primary_block, primary_off) = if block_size == 1024 { (1, 0) } else { (0, 1024) };
        self.patch_super_block_copy(&sb, primary_block, primary_off, 0)?;

        // Backup superblocks live at the first block of each backup group.
        let bg_count = sb.block_group_count();
        let sparse = sb.has_sparse_super();
        for g in 1..bg_count {
            if sparse && !group_has_super_backup(g) {
                continue;
            }
            let group_block = sb.first_data_block + g * sb.blocks_per_group;
            self.patch_super_block_copy(&sb, group_block as usize, 0, g as u16)?;
        }
        Ok(())
    }

    /// Read-modify-write one superblock copy: preserve the on-disk image,
    /// patch the mutable counters, and write it back.
    fn patch_super_block_copy(
        &self,
        sb: &SuperBlock,
        fs_block: usize,
        offset_in_block: usize,
        group_nr: u16,
    ) -> Result<(), FsError> {
        let block_size = self.block_size;
        let mut buf = vec![0u8; block_size];
        self.read_fs_block(fs_block, &mut buf)?;
        sb.patch_into(&mut buf[offset_in_block..offset_in_block + 1024], group_nr, 0);
        self.write_fs_block(fs_block, &buf)?;
        Ok(())
    }

    /// Write the block group descriptor table back to the primary location
    /// and every backup copy.
    pub fn write_block_group_descriptors(&self) -> Result<(), FsError> {
        if self.read_only {
            return Ok(());
        }
        let (first_data_block, blocks_per_group, bg_count, sparse) = {
            let sb = self.super_block.lock();
            (
                sb.first_data_block,
                sb.blocks_per_group,
                sb.block_group_count(),
                sb.has_sparse_super(),
            )
        };
        let bgdt_block = first_data_block as usize + 1;

        // Primary copy.
        self.write_bgdt_at(bgdt_block)?;

        // Backup copies: immediately after the backup superblock in each
        // backup group.
        for g in 1..bg_count {
            if sparse && !group_has_super_backup(g) {
                continue;
            }
            let group_block = (first_data_block + g * blocks_per_group) as usize;
            self.write_bgdt_at(group_block + 1)?;
        }
        Ok(())
    }

    /// Serialize the descriptor table and write it starting at `start_block`,
    /// preserving any trailing padding in the final block.
    fn write_bgdt_at(&self, start_block: usize) -> Result<(), FsError> {
        let groups = self.block_groups.lock();
        let block_size = self.block_size;
        let entry_size = core::mem::size_of::<BlockGroupDescriptor>();
        let total_bytes = groups.len() * entry_size;
        let nblocks = total_bytes.div_ceil(block_size);

        for i in 0..nblocks {
            let fs_block = start_block + i;
            // Read-modify-write so trailing padding is preserved.
            let mut buf = vec![0u8; block_size];
            self.read_fs_block(fs_block, &mut buf)?;

            let block_first = i * block_size;
            for (gi, bg) in groups.iter().enumerate() {
                let goff = gi * entry_size;
                if goff + entry_size <= block_first || goff >= block_first + block_size {
                    continue;
                }
                let in_block = goff - block_first;
                // SAFETY: BlockGroupDescriptor is repr(C, packed) (alignment
                // 1) and exactly `entry_size` bytes.
                let src = unsafe {
                    core::slice::from_raw_parts(
                        (bg as *const BlockGroupDescriptor).cast::<u8>(),
                        entry_size,
                    )
                };
                buf[in_block..in_block + entry_size].copy_from_slice(src);
            }
            self.write_fs_block(fs_block, &buf)?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Inode cache
    // -----------------------------------------------------------------------

    /// Return the (unique, cached) [`Ext2INode`] for `inode_num`, reading it
    /// from disk on a cache miss.
    pub fn get_inode(&self, inode_num: u32) -> Result<Arc<Ext2INode>, FsError> {
        let mut cache = self.inode_cache.lock();
        if let Some(weak) = cache.get(&inode_num)
            && let Some(strong) = weak.upgrade()
        {
            return Ok(strong);
        }
        let fs = self.self_ref.lock().upgrade().ok_or(FsError::IoError)?;
        let inode = Arc::new(Ext2INode::read(&fs, inode_num)?);

        // Opportunistically drop entries whose inode has been freed (their
        // `Weak` no longer upgrades) so the map does not grow unbounded with
        // stale keys.
        if cache.len() >= INODE_CACHE_PRUNE_THRESHOLD {
            cache.retain(|_, w| w.strong_count() > 0);
        }
        cache.insert(inode_num, Arc::downgrade(&inode));
        Ok(inode)
    }

    /// Drop `inode_num` from the inode cache (called when the inode is freed).
    pub fn invalidate_inode(&self, inode_num: u32) {
        self.inode_cache.lock().remove(&inode_num);
    }

    // -----------------------------------------------------------------------
    // Directory accounting
    // -----------------------------------------------------------------------

    /// Adjust `bg_used_dirs_count` for the group owning `inode_num` by
    /// `delta` (+1 on directory creation, -1 on removal).
    pub fn adjust_used_dirs(&self, inode_num: u32, delta: i32) {
        let inodes_per_group = self.super_block.lock().inodes_per_group;
        if inodes_per_group == 0 {
            return;
        }
        let group = ((inode_num - 1) / inodes_per_group) as usize;
        let mut groups = self.block_groups.lock();
        if let Some(bg) = groups.get_mut(group) {
            if delta >= 0 {
                bg.bg_used_dirs_count = bg.bg_used_dirs_count.saturating_add(delta as u16);
            } else {
                bg.bg_used_dirs_count =
                    bg.bg_used_dirs_count.saturating_sub((-delta) as u16);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SuperBlock trait (VFS mount point)
// ---------------------------------------------------------------------------

impl SuperBlockTrait for Ext2Fs {
    fn root_inode(&self) -> Arc<dyn INode> {
        self.get_inode(EXT2_ROOT_INO)
            .expect("ext2 root inode (2) must be readable") as Arc<dyn INode>
    }

    fn sync(&self) {
        // Persist metadata into the cache, then flush the cache to disk.
        let _ = self.write_super_block();
        let _ = self.write_block_group_descriptors();
        let _ = self.cache.lock().sync();
    }

    fn on_mount(&self) {
        if self.read_only {
            return;
        }
        // Clear the "cleanly unmounted" bit and bump the mount count so a
        // crash before unmount is detectable by fsck.
        {
            let mut sb = self.super_block.lock();
            sb.state &= !EXT2_VALID_FS;
            sb.mnt_count = sb.mnt_count.saturating_add(1);
        }
        let _ = self.write_super_block();
        let _ = self.cache.lock().sync();
    }

    fn on_unmount(&self) {
        if !self.read_only {
            // Mark the filesystem cleanly unmounted.
            self.super_block.lock().state |= EXT2_VALID_FS;
        }
        // Flush all metadata and dirty blocks.
        self.sync();
    }

    fn info(&self) -> FsInfo {
        let sb = self.super_block.lock();
        FsInfo {
            total_blocks: sb.blocks_count as usize,
            free_blocks: sb.free_blocks_count as usize,
            total_inodes: sb.inodes_count as usize,
            free_inodes: sb.free_inodes_count as usize,
            block_size: self.block_size,
        }
    }

    fn read_only(&self) -> bool {
        self.read_only
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Reinterpret a byte slice as a reference to `T`.
///
/// # Safety (logical)
///
/// The caller must ensure the slice is large enough and correctly aligned for
/// `T`.  Because `RawSuperBlock` and `BlockGroupDescriptor` are
/// `repr(C, packed)`, alignment is 1 and any byte offset is valid.
fn bytemuck_or_ref<T: Copy>(bytes: &[u8]) -> &T {
    assert!(bytes.len() >= core::mem::size_of::<T>());
    // SAFETY: `T` is `repr(C, packed)` (alignment 1) and the slice is large
    // enough.  We copy to avoid unaligned-reference UB on non-packed reads.
    unsafe { &*(bytes.as_ptr().cast::<T>()) }
}

/// With the `sparse_super` feature, only groups 0, 1, and powers of 3, 5,
/// and 7 keep backup copies of the superblock and block group descriptor
/// table.  Group 0 is the primary and handled separately, so this returns
/// `true` for group 1 and the relevant powers.
fn group_has_super_backup(group: u32) -> bool {
    if group <= 1 {
        return true;
    }
    is_power_of(group, 3) || is_power_of(group, 5) || is_power_of(group, 7)
}

/// `true` if `n` is a non-negative integer power of `base` (`base^k == n`).
fn is_power_of(mut n: u32, base: u32) -> bool {
    if n == 0 {
        return false;
    }
    while n % base == 0 {
        n /= base;
    }
    n == 1
}

/// Parse `count` [`BlockGroupDescriptor`] entries from a byte buffer.
fn parse_block_group_descriptors(buf: &[u8], count: usize) -> Vec<BlockGroupDescriptor> {
    let entry_size = core::mem::size_of::<BlockGroupDescriptor>();
    let mut groups = Vec::with_capacity(count);
    for i in 0..count {
        let offset = i * entry_size;
        let end = offset + entry_size;
        if end > buf.len() {
            break;
        }
        let desc: &BlockGroupDescriptor = bytemuck_or_ref(&buf[offset..end]);
        groups.push(*desc);
    }
    groups
}

// ---------------------------------------------------------------------------
// FileSystemDriver — register with kor_fs at boot
// ---------------------------------------------------------------------------

/// ext2 filesystem driver singleton.
///
/// Register at boot: `kor_fs::register_filesystem(&kor_ext2::EXT2_DRIVER)`.
pub struct Ext2Driver;
pub static EXT2_DRIVER: Ext2Driver = Ext2Driver;

impl kor::FileSystemDriver for Ext2Driver {
    fn name(&self) -> &'static str { "ext2" }
    fn mount(&self, device: Option<Arc<dyn kor::BlockDevice>>) -> Result<Arc<dyn kor::SuperBlock>, FsError> {
        let device = device.ok_or(FsError::InvalidInput)?;
        let fs: Arc<dyn kor::SuperBlock> = Ext2Fs::open(device)?;
        Ok(fs)
    }
}
