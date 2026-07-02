//! ext2 block group descriptor definition.

// ---------------------------------------------------------------------------
// On-disk block group descriptor (32 bytes)
// ---------------------------------------------------------------------------

/// A single block group descriptor, as stored on disk.
///
/// The block group descriptor table starts at the block immediately
/// following the superblock (block 2 when block_size == 1024, or block 1
/// when block_size > 1024).  Each descriptor is exactly 32 bytes.
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct BlockGroupDescriptor {
    /// Block number of the first block of the block bitmap for this group.
    pub bg_block_bitmap: u32,
    /// Block number of the first block of the inode bitmap for this group.
    pub bg_inode_bitmap: u32,
    /// Block number of the first block of the inode table for this group.
    pub bg_inode_table: u32,
    /// Number of free blocks in this group.
    pub bg_free_blocks_count: u16,
    /// Number of free inodes in this group.
    pub bg_free_inodes_count: u16,
    /// Number of inodes allocated to directories in this group.
    pub bg_used_dirs_count: u16,
    /// Padding — reserved for future use (set to zero).
    pub bg_pad: u16,
    /// Reserved for future use (set to zero).
    pub bg_reserved: [u32; 3],
}

// Compile-time assertion: the descriptor must be exactly 32 bytes.
const _: () = assert!(core::mem::size_of::<BlockGroupDescriptor>() == 32);
