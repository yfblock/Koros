//! ext2 superblock definition and parsing.

use crate::FsError;

/// ext2 magic number identifying a valid superblock.
const EXT2_MAGIC: u16 = 0xEF53;

// ---------------------------------------------------------------------------
// Feature flags (ext2 revision 1)
// ---------------------------------------------------------------------------

/// Directory entries record the file type in a dedicated byte.
pub const INCOMPAT_FILETYPE: u32 = 0x0002;

/// Incompatible features Koros knows how to handle.  A filesystem with any
/// *other* incompat bit set must not be mounted.
pub const SUPPORTED_INCOMPAT: u32 = INCOMPAT_FILETYPE;

/// Backup superblocks/GDT are only kept in sparse groups (0, 1, powers of
/// 3/5/7).
pub const RO_COMPAT_SPARSE_SUPER: u32 = 0x0001;
/// Regular-file size is 64-bit (`i_size` low, `i_dir_acl` high).
pub const RO_COMPAT_LARGE_FILE: u32 = 0x0002;

/// Read-only-compatible features Koros can honour while still mounting
/// read-write.  Any other ro_compat bit forces a read-only mount.
pub const SUPPORTED_RO_COMPAT: u32 = RO_COMPAT_SPARSE_SUPER | RO_COMPAT_LARGE_FILE;

/// Filesystem state: cleanly unmounted.
pub const EXT2_VALID_FS: u16 = 0x0001;

// On-disk byte offsets of mutable superblock fields (from the start of the
// 1024-byte superblock), used for read-modify-write persistence.
const OFF_FREE_BLOCKS_COUNT: usize = 12;
const OFF_FREE_INODES_COUNT: usize = 16;
const OFF_MNT_COUNT: usize = 52;
const OFF_WTIME: usize = 48;
const OFF_STATE: usize = 58;
const OFF_BLOCK_GROUP_NR: usize = 90;

// ---------------------------------------------------------------------------
// Raw on-disk superblock (1024 bytes, matches ext2 specification)
// ---------------------------------------------------------------------------

/// On-disk ext2 superblock structure, byte-for-byte compatible with the
/// ext2 specification.  Located at byte offset 1024 from the start of the
/// partition (i.e. the second 1024-byte block, or block 1 when the block
/// size is 1024).
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct RawSuperBlock {
    /// Total number of inodes in the filesystem.
    pub inodes_count: u32,
    /// Total number of blocks in the filesystem.
    pub blocks_count: u32,
    /// Number of blocks reserved for the superuser.
    pub r_blocks_count: u32,
    /// Number of free blocks.
    pub free_blocks_count: u32,
    /// Number of free inodes.
    pub free_inodes_count: u32,
    /// Block number of the block containing the superblock
    /// (0 for 1024-byte block size, 1 otherwise).
    pub first_data_block: u32,
    /// log2(block_size) - 10.  Block size = 1024 << log_block_size.
    pub log_block_size: u32,
    /// log2(fragment_size) - 10 (usually equal to log_block_size).
    pub log_frag_size: u32,
    /// Number of blocks per block group.
    pub blocks_per_group: u32,
    /// Number of fragments per block group.
    pub frags_per_group: u32,
    /// Number of inodes per block group.
    pub inodes_per_group: u32,
    /// Last mount time (POSIX timestamp).
    pub mtime: u32,
    /// Last write time (POSIX timestamp).
    pub wtime: u32,
    /// Number of mounts since the last consistency check.
    pub mnt_count: u16,
    /// Maximum number of mounts before a consistency check is required.
    pub max_mnt_count: u16,
    /// Magic number — must be `0xEF53`.
    pub magic: u16,
    /// Filesystem state: 1 = clean, 2 = errors.
    pub state: u16,
    /// Behaviour when errors are detected.
    pub errors: u16,
    /// Minor revision level.
    pub minor_rev_level: u16,
    /// Time of last consistency check (POSIX timestamp).
    pub lastcheck: u32,
    /// Maximum time between consistency checks (seconds).
    pub checkinterval: u32,
    /// Creator OS: 0 = Linux, 1 = Hurd, 2 = MASIX, 3 = FreeBSD, 4 = Lites.
    pub creator_os: u32,
    /// Major revision level.
    pub rev_level: u32,
    /// Default user ID for reserved blocks.
    pub def_resuid: u16,
    /// Default group ID for reserved blocks.
    pub def_resgid: u16,
    // -- EXT2_DYNAMIC_REV (revision 1) fields, offset 84 onwards --
    /// First non-reserved inode number.
    pub first_ino: u32,
    /// Size of each inode structure in bytes (>= 128).
    pub inode_size: u16,
    /// Block group number hosting this superblock.
    pub block_group_nr: u16,
    /// Compatible feature flags.
    pub feature_compat: u32,
    /// Incompatible feature flags.
    pub feature_incompat: u32,
    /// Read-only compatible feature flags.
    pub feature_ro_compat: u32,
    /// 128-bit filesystem UUID.
    pub uuid: [u8; 16],
    /// Volume name (null-terminated, up to 16 bytes).
    pub volume_name: [u8; 16],
    /// Path where the filesystem was last mounted (null-terminated, 64 bytes).
    pub last_mounted: [u8; 64],
    /// Algorithm usage bitmap.
    pub algo_bitmap: u32,
    // Fields beyond this point are version-dependent and not needed for
    // basic superblock parsing; they are omitted to keep the struct lean.
}

// The on-disk fields up to `s_algo_bitmap` occupy exactly 204 bytes:
//   0..84  base (rev 0) fields, then dynamic-rev fields:
//   84 first_ino(4) 88 inode_size(2) 90 block_group_nr(2)
//   92 feature_compat(4) 96 feature_incompat(4) 100 feature_ro_compat(4)
//   104 uuid(16) 120 volume_name(16) 136 last_mounted(64) 200 algo_bitmap(4)
const _: () = assert!(core::mem::size_of::<RawSuperBlock>() == 204);

// ---------------------------------------------------------------------------
// Parsed superblock (a subset of fields relevant to basic ext2 operation)
// ---------------------------------------------------------------------------

/// A parsed, validated ext2 superblock.
///
/// Contains only the fields needed for read-only filesystem traversal.
/// Mutable or version-3 fields are omitted for now.
#[derive(Clone, Debug)]
pub struct SuperBlock {
    /// Total number of inodes.
    pub inodes_count: u32,
    /// Total number of blocks.
    pub blocks_count: u32,
    /// Number of free blocks.
    pub free_blocks_count: u32,
    /// Number of free inodes.
    pub free_inodes_count: u32,
    /// Block number of the first data block (0 or 1 depending on block size).
    pub first_data_block: u32,
    /// log2(block_size) - 10.
    pub log_block_size: u32,
    /// Number of blocks per block group.
    pub blocks_per_group: u32,
    /// Number of inodes per block group.
    pub inodes_per_group: u32,
    /// Size of an on-disk inode structure in bytes.
    pub inode_size: u16,
    /// Magic number (should always be `0xEF53`).
    pub magic: u16,
    /// Major revision level.
    pub rev_level: u32,
    /// First non-reserved inode number (11 for rev 0).
    pub first_ino: u32,
    /// Compatible feature flags.
    pub feature_compat: u32,
    /// Incompatible feature flags.
    pub feature_incompat: u32,
    /// Read-only compatible feature flags.
    pub feature_ro_compat: u32,
    /// Filesystem state bits (bit 0 = cleanly unmounted).
    pub state: u16,
    /// Number of mounts since the last consistency check.
    pub mnt_count: u16,
}

/// First inode number in a revision-0 filesystem.
const EXT2_GOOD_OLD_FIRST_INO: u32 = 11;
/// Inode size in a revision-0 filesystem.
const EXT2_GOOD_OLD_INODE_SIZE: u16 = 128;
/// Largest accepted `log_block_size` (block size ≤ 64 KiB); also guards the
/// `1024 << log_block_size` shift against overflow.
const MAX_LOG_BLOCK_SIZE: u32 = 6;

impl SuperBlock {
    /// Parse a raw on-disk superblock into a validated [`SuperBlock`].
    ///
    /// Returns [`FsError::InvalidInput`] if the magic number is wrong.
    pub fn from_raw(raw: &RawSuperBlock) -> Result<Self, FsError> {
        if raw.magic != EXT2_MAGIC {
            return Err(FsError::InvalidInput);
        }

        // Reject obviously-corrupt geometry before any arithmetic that could
        // overflow or divide by zero.
        //
        // `block_size == 1024 << log_block_size`; cap the shift so it cannot
        // overflow and stays within the block sizes ext2 actually uses
        // (1 KiB … 64 KiB).
        if raw.log_block_size > MAX_LOG_BLOCK_SIZE
            || raw.blocks_per_group == 0
            || raw.inodes_per_group == 0
            || raw.blocks_count == 0
            || raw.inodes_count == 0
        {
            return Err(FsError::InvalidInput);
        }
        let block_size: usize = 1024usize << raw.log_block_size;

        // Revision 0 filesystems have fixed inode size / first inode and no
        // dynamic-rev feature fields (they read as zero, which is correct).
        let (inode_size, first_ino) = if raw.rev_level == 0 {
            (EXT2_GOOD_OLD_INODE_SIZE, EXT2_GOOD_OLD_FIRST_INO)
        } else {
            let sz = if raw.inode_size == 0 {
                EXT2_GOOD_OLD_INODE_SIZE
            } else {
                raw.inode_size
            };
            let fi = if raw.first_ino == 0 {
                EXT2_GOOD_OLD_FIRST_INO
            } else {
                raw.first_ino
            };
            (sz, fi)
        };

        // Inode size must be a power of two, at least the historical minimum,
        // and no larger than a filesystem block.
        if inode_size < EXT2_GOOD_OLD_INODE_SIZE
            || !inode_size.is_power_of_two()
            || inode_size as usize > block_size
        {
            return Err(FsError::InvalidInput);
        }

        Ok(Self {
            inodes_count: raw.inodes_count,
            blocks_count: raw.blocks_count,
            free_blocks_count: raw.free_blocks_count,
            free_inodes_count: raw.free_inodes_count,
            first_data_block: raw.first_data_block,
            log_block_size: raw.log_block_size,
            blocks_per_group: raw.blocks_per_group,
            inodes_per_group: raw.inodes_per_group,
            inode_size,
            magic: raw.magic,
            rev_level: raw.rev_level,
            first_ino,
            feature_compat: raw.feature_compat,
            feature_incompat: raw.feature_incompat,
            feature_ro_compat: raw.feature_ro_compat,
            state: raw.state,
            mnt_count: raw.mnt_count,
        })
    }

    /// Compute the filesystem block size in bytes: `1024 << log_block_size`.
    pub fn block_size(&self) -> usize {
        1024 << self.log_block_size
    }

    /// Compute the number of block groups in the filesystem.
    pub fn block_group_count(&self) -> u32 {
        self.blocks_count.div_ceil(self.blocks_per_group)
    }

    /// `true` if directory entries carry an explicit file-type byte.
    pub fn has_filetype(&self) -> bool {
        self.feature_incompat & INCOMPAT_FILETYPE != 0
    }

    /// `true` if regular files may use the 64-bit `large_file` size encoding.
    pub fn has_large_file(&self) -> bool {
        self.feature_ro_compat & RO_COMPAT_LARGE_FILE != 0
    }

    /// `true` if only sparse groups keep superblock/GDT backups.
    pub fn has_sparse_super(&self) -> bool {
        self.feature_ro_compat & RO_COMPAT_SPARSE_SUPER != 0
    }

    /// Incompatible feature bits that Koros does not understand.
    pub fn unsupported_incompat(&self) -> u32 {
        self.feature_incompat & !SUPPORTED_INCOMPAT
    }

    /// Read-only-compatible feature bits that Koros does not understand.
    pub fn unsupported_ro_compat(&self) -> u32 {
        self.feature_ro_compat & !SUPPORTED_RO_COMPAT
    }

    /// Patch the mutable counters (and `block_group_nr`/`wtime`) into a raw
    /// 1024-byte superblock image for read-modify-write persistence.
    ///
    /// `block_group_nr` is the group number this copy lives in (0 for the
    /// primary superblock, `g` for a backup in group `g`).
    pub fn patch_into(&self, buf: &mut [u8], block_group_nr: u16, wtime: u32) {
        buf[OFF_FREE_BLOCKS_COUNT..OFF_FREE_BLOCKS_COUNT + 4]
            .copy_from_slice(&self.free_blocks_count.to_le_bytes());
        buf[OFF_FREE_INODES_COUNT..OFF_FREE_INODES_COUNT + 4]
            .copy_from_slice(&self.free_inodes_count.to_le_bytes());
        buf[OFF_MNT_COUNT..OFF_MNT_COUNT + 2].copy_from_slice(&self.mnt_count.to_le_bytes());
        buf[OFF_WTIME..OFF_WTIME + 4].copy_from_slice(&wtime.to_le_bytes());
        buf[OFF_STATE..OFF_STATE + 2].copy_from_slice(&self.state.to_le_bytes());
        // block_group_nr only exists in the dynamic-rev header.
        if self.rev_level > 0 {
            buf[OFF_BLOCK_GROUP_NR..OFF_BLOCK_GROUP_NR + 2]
                .copy_from_slice(&block_group_nr.to_le_bytes());
        }
    }
}

// ---------------------------------------------------------------------------
// Compile-time size check
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;

    #[test]
    fn raw_superblock_size() {
        // The ext2 superblock is 1024 bytes on disk, but we only define
        // the first 84 bytes that are version-independent.
        assert!(size_of::<RawSuperBlock>() >= 84);
    }
}
