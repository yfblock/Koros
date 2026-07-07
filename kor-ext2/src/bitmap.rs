//! ext2 block/inode bitmap operations.
//!
//! Provides a [`Bitmap`] wrapper for manipulating the on-disk bitmaps used
//! by ext2 to track free blocks and inodes within each block group, plus
//! [`Ext2Fs`] methods for allocating and deallocating filesystem blocks.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use kor::FsError;

use super::Ext2Fs;

// ---------------------------------------------------------------------------
// Bitmap — a thin wrapper over a byte vector with bit-level access
// ---------------------------------------------------------------------------

/// A bitmap backed by a `Vec<u8>`, where bit 0 of byte 0 is the LSB.
///
/// Used to represent ext2 block and inode bitmaps on disk.  The `size` field
/// tracks the number of *valid* bits (the backing buffer may be slightly
/// larger due to byte-alignment).
pub struct Bitmap {
    /// Raw bitmap data, one bit per block/inode.
    pub(super) data: Vec<u8>,
    /// Number of valid bits in the bitmap.
    size: usize,
}

impl Bitmap {
    /// Wrap an existing byte buffer as a bitmap covering `size` bits.
    pub fn from_bytes(data: Vec<u8>, size: usize) -> Self {
        Self { data, size }
    }

    /// Return `true` if bit `index` is set (allocated).
    ///
    /// Returns [`FsError::InvalidInput`] if `index` is out of range (e.g. a
    /// corrupt on-disk value) rather than panicking.
    pub fn get(&self, index: usize) -> Result<bool, FsError> {
        let byte = index / 8;
        if index >= self.size || byte >= self.data.len() {
            return Err(FsError::InvalidInput);
        }
        let bit = index % 8;
        Ok((self.data[byte] >> bit) & 1 != 0)
    }

    /// Set bit `index` (mark as allocated).
    ///
    /// Returns [`FsError::InvalidInput`] if `index` is out of range.
    pub fn set(&mut self, index: usize) -> Result<(), FsError> {
        let byte = index / 8;
        if index >= self.size || byte >= self.data.len() {
            return Err(FsError::InvalidInput);
        }
        let bit = index % 8;
        self.data[byte] |= 1 << bit;
        Ok(())
    }

    /// Clear bit `index` (mark as free).
    ///
    /// Returns [`FsError::InvalidInput`] if `index` is out of range.
    pub fn clear(&mut self, index: usize) -> Result<(), FsError> {
        let byte = index / 8;
        if index >= self.size || byte >= self.data.len() {
            return Err(FsError::InvalidInput);
        }
        let bit = index % 8;
        self.data[byte] &= !(1 << bit);
        Ok(())
    }

    /// Find the index of the first clear (free) bit, or `None` if all bits
    /// are set.
    pub fn find_first_clear(&self) -> Option<usize> {
        for (byte_idx, &byte) in self.data.iter().enumerate() {
            if byte != 0xFF {
                for bit in 0..8 {
                    let index = byte_idx * 8 + bit;
                    if index >= self.size {
                        return None;
                    }
                    if (byte >> bit) & 1 == 0 {
                        return Some(index);
                    }
                }
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Block allocation on Ext2Fs
// ---------------------------------------------------------------------------

impl Ext2Fs {
    /// Allocate a free filesystem block and return its block number.
    ///
    /// Scans each block group's block bitmap for the first free bit, marks
    /// it as allocated, writes the bitmap back to disk, and decrements the
    /// free-block counts in both the block group descriptor and the
    /// superblock.
    ///
    /// Returns [`FsError::NoSpace`] if no free block is available.
    pub fn alloc_block(&self) -> Result<u32, FsError> {
        self.alloc_block_goal(0)
    }

    /// Allocate a free block, preferring `goal_group` and its neighbours for
    /// locality before falling back to a wrap-around scan of all groups.
    pub fn alloc_block_goal(&self, goal_group: u32) -> Result<u32, FsError> {
        if self.is_read_only() {
            return Err(FsError::ReadOnly);
        }
        let block_size = self.block_size();
        let mut result: Option<u32> = None;

        {
            // Hold both locks for the entire allocation to prevent races.
            let mut block_groups = self.block_groups.lock();
            let mut super_block = self.super_block.lock();
            let blocks_per_group = super_block.blocks_per_group;
            let first_data_block = super_block.first_data_block;
            let blocks_count = super_block.blocks_count;
            let ngroups = block_groups.len();

            for step in 0..ngroups {
                let group_idx = ((goal_group as usize) + step) % ngroups;
                let bg = &mut block_groups[group_idx];
                if bg.bg_free_blocks_count == 0 {
                    continue;
                }

                // The final group may be shorter than `blocks_per_group`;
                // cap the bitmap so we never hand out a block past the end of
                // the device (mirrors the inode allocator's last-group logic).
                let group_first = first_data_block + group_idx as u32 * blocks_per_group;
                let blocks_in_group =
                    core::cmp::min(blocks_per_group, blocks_count - group_first) as usize;

                let bitmap_block = bg.bg_block_bitmap as usize;
                let mut bitmap_buf = vec![0u8; block_size];
                self.read_fs_block(bitmap_block, &mut bitmap_buf)?;

                let mut bitmap = Bitmap::from_bytes(bitmap_buf, blocks_in_group);
                let local_idx = match bitmap.find_first_clear() {
                    Some(idx) => idx,
                    None => continue,
                };

                bitmap.set(local_idx)?;
                self.write_fs_block(bitmap_block, &bitmap.data)?;

                let block_id = group_idx as u32 * blocks_per_group
                    + local_idx as u32
                    + super_block.first_data_block;

                bg.bg_free_blocks_count -= 1;
                super_block.free_blocks_count -= 1;
                result = Some(block_id);
                break;
            }
        }

        // The bitmap block was written through the cache above; the superblock
        // and BGDT free counts are updated in memory and persisted lazily by
        // `sync()` / unmount (an interrupted mount leaves the bitmaps correct
        // and the counts recoverable by fsck), avoiding per-allocation
        // serialization of the whole descriptor table.
        result.ok_or(FsError::NoSpace)
    }

    /// Free a previously allocated filesystem block.
    ///
    /// Calculates which block group owns `block_id`, clears the
    /// corresponding bit in the block bitmap, writes it back to disk, and
    /// increments the free-block counts in the block group descriptor and
    /// the superblock.
    pub fn free_block(&self, block_id: u32) -> Result<(), FsError> {
        if self.is_read_only() {
            return Err(FsError::ReadOnly);
        }
        let block_size = self.block_size();

        // Read immutable superblock fields first (short lock).
        let (blocks_per_group, first_data_block) = {
            let sb = self.super_block.lock();
            (sb.blocks_per_group, sb.first_data_block)
        };

        // Compute group index and local bit index.
        let local_id = block_id
            .checked_sub(first_data_block)
            .ok_or(FsError::InvalidInput)?;
        let group_idx = local_id / blocks_per_group;
        let local_idx = (local_id % blocks_per_group) as usize;

        {
            let mut block_groups = self.block_groups.lock();
            let mut super_block = self.super_block.lock();

            let bg = block_groups
                .get_mut(group_idx as usize)
                .ok_or(FsError::InvalidInput)?;

            let bitmap_block = bg.bg_block_bitmap as usize;
            let mut bitmap_buf = vec![0u8; block_size];
            self.read_fs_block(bitmap_block, &mut bitmap_buf)?;

            let mut bitmap = Bitmap::from_bytes(bitmap_buf, blocks_per_group as usize);
            bitmap.clear(local_idx)?;
            self.write_fs_block(bitmap_block, &bitmap.data)?;

            bg.bg_free_blocks_count += 1;
            super_block.free_blocks_count += 1;
        }
        // Counts persisted lazily by `sync()` (see `alloc_block_goal`).
        Ok(())
    }
}

impl Ext2Fs {
    /// Allocate a free inode and return its 1-based inode number.
    ///
    /// Scans each block group's inode bitmap for a free bit, marks it
    /// allocated, writes the bitmap back to disk, and decrements the
    /// free-inode counts in both the block group descriptor and the
    /// superblock.
    ///
    /// Returns [`FsError::NoSpace`] if no free inode is available.
    pub fn alloc_inode(&self) -> Result<u32, FsError> {
        if self.is_read_only() {
            return Err(FsError::ReadOnly);
        }
        let block_size = self.block_size();
        let mut result: Option<u32> = None;

        {
            let mut block_groups = self.block_groups.lock();
            let mut super_block = self.super_block.lock();
            let inodes_per_group = super_block.inodes_per_group;
            let bg_count = super_block.block_group_count();

            for (group_idx, bg) in block_groups.iter_mut().enumerate() {
                if bg.bg_free_inodes_count == 0 {
                    continue;
                }

                let bitmap_block = bg.bg_inode_bitmap as usize;
                let mut bitmap_buf = vec![0u8; block_size];
                self.read_fs_block(bitmap_block, &mut bitmap_buf)?;

                let inodes_in_this_group = if group_idx as u32 == bg_count - 1 {
                    super_block.inodes_count - (bg_count - 1) * inodes_per_group
                } else {
                    inodes_per_group
                };

                let mut bitmap = Bitmap::from_bytes(bitmap_buf, inodes_in_this_group as usize);
                let local_idx = match bitmap.find_first_clear() {
                    Some(idx) => idx,
                    None => continue,
                };

                bitmap.set(local_idx)?;
                self.write_fs_block(bitmap_block, &bitmap.data)?;

                let inode_num = group_idx as u32 * inodes_per_group + local_idx as u32 + 1;
                bg.bg_free_inodes_count -= 1;
                super_block.free_inodes_count -= 1;
                result = Some(inode_num);
                break;
            }
        }

        // Counts persisted lazily by `sync()` (see `alloc_block_goal`).
        result.ok_or(FsError::NoSpace)
    }

    /// Free the inode `inode_num` (1-based), returning it to the bitmap.
    ///
    /// Clears the corresponding bit in the inode bitmap and increments
    /// the free-inode counts in the block group descriptor and superblock.
    pub fn free_inode(&self, inode_num: u32) -> Result<(), FsError> {
        if inode_num == 0 {
            return Err(FsError::InvalidInput);
        }
        if self.is_read_only() {
            return Err(FsError::ReadOnly);
        }

        let inodes_per_group = {
            let sb = self.super_block.lock();
            sb.inodes_per_group
        };

        let group_idx = (inode_num - 1) / inodes_per_group;
        let local_idx = ((inode_num - 1) % inodes_per_group) as usize;

        {
            let mut block_groups = self.block_groups.lock();
            let mut super_block = self.super_block.lock();

            let bg = block_groups
                .get_mut(group_idx as usize)
                .ok_or(FsError::InvalidInput)?;

            let bitmap_block = bg.bg_inode_bitmap as usize;
            let mut bitmap_buf = vec![0u8; self.block_size()];
            self.read_fs_block(bitmap_block, &mut bitmap_buf)?;

            let mut bitmap =
                Bitmap::from_bytes(bitmap_buf, super_block.inodes_per_group as usize);
            bitmap.clear(local_idx)?;
            self.write_fs_block(bitmap_block, &bitmap.data)?;

            bg.bg_free_inodes_count += 1;
            super_block.free_inodes_count += 1;
        }

        // Drop any cached in-memory copy of the freed inode.
        self.invalidate_inode(inode_num);
        // Counts persisted lazily by `sync()` (see `alloc_block_goal`).
        Ok(())
    }

    /// Initialize the on-disk inode `inode_num` with the given `mode`.
    ///
    /// Zeroes the entire 128-byte inode structure, sets `i_mode` to `mode`
    /// and `i_links_count` to 1.
    pub fn init_inode(&self, inode_num: u32, mode: u16) -> Result<(), FsError> {
        if inode_num == 0 {
            return Err(FsError::InvalidInput);
        }

        let (inodes_per_group, inode_size) = {
            let sb = self.super_block.lock();
            (sb.inodes_per_group, sb.inode_size as usize)
        };

        let group_idx = (inode_num - 1) / inodes_per_group;
        let local_idx = (inode_num - 1) % inodes_per_group;

        let inode_table_block = {
            let groups = self.block_groups.lock();
            let bg = groups
                .get(group_idx as usize)
                .ok_or(FsError::InvalidInput)?;
            bg.bg_inode_table as usize
        };

        let offset_in_table = (local_idx as usize) * inode_size;
        let block_size = self.block_size();
        let fs_block = inode_table_block + offset_in_table / block_size;
        let offset_in_block = offset_in_table % block_size;

        let mut block_buf = vec![0u8; block_size];
        self.read_fs_block(fs_block, &mut block_buf)?;

        let inode_bytes = &mut block_buf[offset_in_block..offset_in_block + inode_size];
        for b in inode_bytes.iter_mut() {
            *b = 0;
        }

        // Set i_mode (offset 0, 2 bytes LE) and i_links_count (offset 26, 2 bytes LE).
        inode_bytes[0..2].copy_from_slice(&mode.to_le_bytes());
        inode_bytes[26..28].copy_from_slice(&1u16.to_le_bytes());

        self.write_fs_block(fs_block, &block_buf)?;

        Ok(())
    }
}
