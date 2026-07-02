//! ext2 directory entry parsing, iteration, lookup, and listing.

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::ptr;

use crate::fs::{FsError, FileType, INode};

use super::inode::Ext2INode;

// ---------------------------------------------------------------------------
// Directory entry file type constants (ext2 specification, §4, table 4.1)
// ---------------------------------------------------------------------------

/// Unknown file type.
pub const TYPE_UNKNOWN: u8 = 0;
/// Regular file.
pub const TYPE_REGULAR: u8 = 1;
/// Directory.
pub const TYPE_DIRECTORY: u8 = 2;
/// Character device.
pub const TYPE_CHAR_DEVICE: u8 = 3;
/// Block device.
pub const TYPE_BLOCK_DEVICE: u8 = 4;
/// FIFO (named pipe).
pub const TYPE_FIFO: u8 = 5;
/// Unix-domain socket.
pub const TYPE_SOCKET: u8 = 6;
/// Symbolic link.
pub const TYPE_SYMLINK: u8 = 7;

/// Fixed header size of an ext2 directory entry (bytes before the name).
const DIR_ENTRY_HEADER_SIZE: usize = 8;

// ---------------------------------------------------------------------------
// On-disk directory entry (dynamically sized)
// ---------------------------------------------------------------------------

/// An ext2 directory entry with a variable-length name.
///
/// On-disk layout:
///   `[0..4]`  inode: u32   — inode number (0 = unused slot)
///   `[4..6]`  rec_len: u16 — total record length including padding
///   `[6]`     name_len: u8 — length of the name in bytes
///   `[7]`     file_type: u8
///   `[8..]`   name: [u8]   — `name_len` bytes, NOT null-terminated
///
/// This is a DST ([dynamically sized type]) whose trailing `[u8]` slice
/// covers exactly `name_len` bytes.
#[repr(C, packed)]
pub struct DirEntry {
    pub inode: u32,
    pub rec_len: u16,
    pub name_len: u8,
    pub file_type: u8,
    pub name: [u8],
}

impl DirEntry {
    /// Parse a directory entry from the front of `data`.
    ///
    /// Returns `Some((entry, remaining))` on success, where `remaining` is the
    /// byte slice starting after this entry's `rec_len`.  Returns `None` when:
    /// - `data` is shorter than the 8-byte header
    /// - the entry is unused (`inode == 0`)
    /// - `rec_len` is smaller than the header or larger than `data`
    /// - `name_len` exceeds the space available in `rec_len`
    pub fn from_bytes(data: &[u8]) -> Option<(&DirEntry, &[u8])> {
        if data.len() < DIR_ENTRY_HEADER_SIZE {
            return None;
        }

        // Read header fields from potentially-unaligned memory.
        let inode = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let rec_len = u16::from_le_bytes([data[4], data[5]]) as usize;
        let name_len = data[6] as usize;

        // Structural validation.
        if rec_len < DIR_ENTRY_HEADER_SIZE || rec_len > data.len() {
            return None;
        }
        if DIR_ENTRY_HEADER_SIZE + name_len > rec_len {
            return None;
        }
        // Skip unused (deleted) entries.
        if inode == 0 {
            return None;
        }

        // SAFETY: DirEntry is repr(C, packed) — alignment is 1.
        // The byte range `[..header+name_len]` is large enough for the DST,
        // and the trailing [u8] slice has exactly `name_len` elements.
        let entry_bytes = &data[..DIR_ENTRY_HEADER_SIZE + name_len];
        let entry: &DirEntry = unsafe { &*(entry_bytes as *const [u8] as *const DirEntry) };

        Some((entry, &data[rec_len..]))
    }

    // -- safe field accessors (packed struct → use ptr::read_unaligned) ------

    /// Inode number of this entry.
    #[inline]
    pub fn inode_num(&self) -> u32 {
        // SAFETY: packed u32 field; addr_of! avoids creating a reference
        // to a potentially-unaligned field.
        unsafe { ptr::read_unaligned(ptr::addr_of!(self.inode)) }
    }

    /// Record length in bytes (including padding).
    #[inline]
    pub fn rec_len(&self) -> u16 {
        unsafe { ptr::read_unaligned(ptr::addr_of!(self.rec_len)) }
    }

    /// Name length in bytes.
    #[inline]
    pub fn name_len(&self) -> u8 {
        // u8 — alignment 1, safe to read through a reference.
        self.name_len
    }

    /// Raw file-type byte from the directory entry.
    #[inline]
    pub fn file_type_byte(&self) -> u8 {
        self.file_type
    }

    /// The entry name as a byte slice (not null-terminated).
    pub fn name_bytes(&self) -> &[u8] {
        &self.name[..self.name_len as usize]
    }

    /// The entry name as a UTF-8 string, or `None` if it is not valid UTF-8.
    pub fn name_str(&self) -> Option<&str> {
        core::str::from_utf8(self.name_bytes()).ok()
    }

    /// Map the on-disk file-type byte to the VFS [`FileType`].
    pub fn vfs_file_type(&self) -> FileType {
        match self.file_type {
            TYPE_REGULAR => FileType::Regular,
            TYPE_DIRECTORY => FileType::Directory,
            TYPE_CHAR_DEVICE => FileType::CharDevice,
            TYPE_BLOCK_DEVICE => FileType::BlockDevice,
            TYPE_SYMLINK => FileType::Symlink,
            _ => FileType::Regular,
        }
    }
}

// ---------------------------------------------------------------------------
// Directory iterator
// ---------------------------------------------------------------------------

/// Iterator over the directory entries of an ext2 directory inode.
///
/// The entire directory is read into memory up-front (directories are
/// typically small — a few KiB).  The iterator then walks entries by
/// parsing [`DirEntry`] headers from the buffered data.
pub struct DirIterator {
    /// Buffered copy of the directory contents.
    data: Vec<u8>,
    /// Current byte offset within `data`.
    offset: usize,
}

impl DirIterator {
    /// Create a new iterator over the directory entries of `inode`.
    ///
    /// Returns [`FsError::NotDirectory`] if `inode` is not a directory.
    pub fn new(inode: &Ext2INode) -> Result<Self, FsError> {
        let mode = inode.raw().i_mode;
        if mode & 0xF000 != 0x4000 {
            return Err(FsError::NotDirectory);
        }

        let dir_size = inode.raw().i_size as usize;
        let mut data = vec![0u8; dir_size];
        if dir_size > 0 {
            inode.read_at(0, &mut data)?;
        }

        Ok(Self { data, offset: 0 })
    }
}

impl Iterator for DirIterator {
    /// Each item is `(inode_number, file_type_byte, name)`.
    type Item = (u32, u8, String);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.offset >= self.data.len() {
                return None;
            }

            let remaining = &self.data[self.offset..];

            match DirEntry::from_bytes(remaining) {
                Some((entry, _rest)) => {
                    let ino = entry.inode_num();
                    let ft = entry.file_type_byte();
                    let name = entry.name_str()?.to_string();
                    let rec = entry.rec_len() as usize;
                    self.offset += rec;
                    return Some((ino, ft, name));
                }
                None => {
                    // Unused or malformed entry — skip it.
                    // Try to read rec_len from the raw header to advance.
                    if remaining.len() < DIR_ENTRY_HEADER_SIZE {
                        // Not enough bytes for even a header — done.
                        return None;
                    }
                    let rec_len =
                        u16::from_le_bytes([remaining[4], remaining[5]]) as usize;
                    if rec_len < DIR_ENTRY_HEADER_SIZE {
                        // Malformed — step forward by one header size.
                        self.offset += DIR_ENTRY_HEADER_SIZE;
                    } else {
                        self.offset += rec_len;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Directory lookup and listing helpers
// ---------------------------------------------------------------------------

/// Look up a child entry by `name` in the directory `inode`.
///
/// Returns the inode number of the matching entry, or
/// [`FsError::NotFound`] / [`FsError::NotDirectory`].
pub fn lookup(inode: &Ext2INode, name: &str) -> Result<u32, FsError> {
    let iter = DirIterator::new(inode)?;
    for (ino, _ft, entry_name) in iter {
        if entry_name == name {
            return Ok(ino);
        }
    }
    Err(FsError::NotFound)
}

/// List all entries in the directory `inode`.
///
/// Returns a vector of `(name, inode_number)` pairs.
pub fn list(inode: &Ext2INode) -> Result<Vec<(String, u32)>, FsError> {
    let iter = DirIterator::new(inode)?;
    let mut entries = Vec::new();
    for (ino, _ft, name) in iter {
        entries.push((name, ino));
    }
    Ok(entries)
}

// ---------------------------------------------------------------------------
// Directory mutation helpers
// ---------------------------------------------------------------------------

/// Write a directory entry into `buf` at `offset`.
///
/// Only writes the header + name; the caller is responsible for setting
/// `rec_len` to the correct value before calling.
fn write_dir_entry_in_buf(
    buf: &mut [u8],
    offset: usize,
    ino: u32,
    name_len: u8,
    file_type: u8,
    name: &[u8],
    rec_len: u16,
) {
    buf[offset..offset + 4].copy_from_slice(&ino.to_le_bytes());
    buf[offset + 4..offset + 6].copy_from_slice(&rec_len.to_le_bytes());
    buf[offset + 6] = name_len;
    buf[offset + 7] = file_type;
    let name_start = offset + DIR_ENTRY_HEADER_SIZE;
    buf[name_start..name_start + name.len()].copy_from_slice(name);
}

/// Padded size of a directory entry for a name of `name_len` bytes.
///
/// Entries are 4-byte aligned with a minimum size of 12 bytes.
fn entry_size(name_len: usize) -> usize {
    ((DIR_ENTRY_HEADER_SIZE + name_len + 3) & !3).max(12)
}

/// Add a new directory entry to the directory `inode`.
///
/// Reuses a deleted (inode=0) slot if one is large enough, otherwise
/// appends at the end of the directory (allocating a new block if needed).
pub fn add_dir_entry(
    inode: &Ext2INode,
    name: &str,
    child_inode: u32,
    file_type: u8,
) -> Result<(), FsError> {
    let name_bytes = name.as_bytes();
    let name_len = name_bytes.len();
    if name_len == 0 || name_len > 255 {
        return Err(FsError::InvalidInput);
    }

    // Modifying a directory linearly: downgrade it from htree first so a
    // Linux reader does not navigate through a now-stale index.
    inode.demote_from_htree()?;

    // Without the FILETYPE feature, byte 7 of a dirent is the high byte of a
    // 16-bit name_len (always 0 for names < 256), not a file-type — so force
    // it to 0 to avoid corrupting the entry for such filesystems.
    let file_type = if inode.fs().has_filetype() {
        file_type
    } else {
        0
    };

    let needed = entry_size(name_len);

    let block_size = inode.fs().block_size();
    let dir_size = inode.raw().i_size as usize;

    if dir_size > 0 {
        let mut data = vec![0u8; dir_size];
        inode.read_at(0, &mut data)?;

        let mut offset = 0;
        let mut last_entry_start = 0usize;
        let mut last_entry_actual = 0usize;

        while offset + DIR_ENTRY_HEADER_SIZE <= data.len() {
            let ent_ino = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            let rec_len =
                u16::from_le_bytes([data[offset + 4], data[offset + 5]]) as usize;
            let ent_name_len = data[offset + 6] as usize;

            if rec_len < DIR_ENTRY_HEADER_SIZE || offset + rec_len > data.len() {
                break;
            }

            let actual = entry_size(ent_name_len);

            if ent_ino == 0 && rec_len >= needed {
                let leftover = rec_len - needed;
                let rec = if leftover >= DIR_ENTRY_HEADER_SIZE {
                    needed as u16
                } else {
                    rec_len as u16
                };
                write_dir_entry_in_buf(
                    &mut data,
                    offset,
                    child_inode,
                    name_len as u8,
                    file_type,
                    name_bytes,
                    rec,
                );
                if leftover >= DIR_ENTRY_HEADER_SIZE {
                    let fo = offset + needed;
                    data[fo..fo + 4].copy_from_slice(&0u32.to_le_bytes());
                    data[fo + 4..fo + 6]
                        .copy_from_slice(&(leftover as u16).to_le_bytes());
                }

                let block_start = (offset / block_size) * block_size;
                inode.write_at(block_start, &data[block_start..block_start + block_size])?;
                return Ok(());
            }

            last_entry_start = offset;
            last_entry_actual = actual;
            offset += rec_len;
        }

        if last_entry_actual > 0 {
            let last_rec = u16::from_le_bytes([
                data[last_entry_start + 4],
                data[last_entry_start + 5],
            ]) as usize;
            let extra = last_rec - last_entry_actual;

            if extra >= needed {
                data[last_entry_start + 4..last_entry_start + 6]
                    .copy_from_slice(&(last_entry_actual as u16).to_le_bytes());

                let new_off = last_entry_start + last_entry_actual;
                write_dir_entry_in_buf(
                    &mut data,
                    new_off,
                    child_inode,
                    name_len as u8,
                    file_type,
                    name_bytes,
                    extra as u16,
                );

                let block_start = (last_entry_start / block_size) * block_size;
                inode.write_at(block_start, &data[block_start..block_start + block_size])?;
                return Ok(());
            }
        }
    }

    let mut new_block = vec![0u8; block_size];
    write_dir_entry_in_buf(
        &mut new_block,
        0,
        child_inode,
        name_len as u8,
        file_type,
        name_bytes,
        block_size as u16,
    );

    let write_offset = (dir_size / block_size) * block_size;
    inode.write_at(write_offset, &new_block)?;

    Ok(())
}

/// Remove the directory entry named `name` from the directory `inode`.
///
/// Marks the entry as deleted (inode=0) and coalesces with an adjacent
/// free successor if one exists.
pub fn remove_dir_entry(inode: &Ext2INode, name: &str) -> Result<(), FsError> {
    // Removing an entry also mutates the directory, so downgrade any htree
    // index to a plain linear layout first.
    inode.demote_from_htree()?;

    let block_size = inode.fs().block_size();
    let dir_size = inode.raw().i_size as usize;
    if dir_size == 0 {
        return Err(FsError::NotFound);
    }

    let mut data = vec![0u8; dir_size];
    inode.read_at(0, &mut data)?;

    let mut offset = 0;
    while offset + DIR_ENTRY_HEADER_SIZE <= data.len() {
        let ent_ino = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]);
        let rec_len =
            u16::from_le_bytes([data[offset + 4], data[offset + 5]]) as usize;
        let name_len = data[offset + 6] as usize;

        if rec_len < DIR_ENTRY_HEADER_SIZE || offset + rec_len > data.len() {
            break;
        }

        if ent_ino != 0
            && name_len == name.len()
            && data[offset + DIR_ENTRY_HEADER_SIZE
                ..offset + DIR_ENTRY_HEADER_SIZE + name_len]
                == *name.as_bytes()
        {
            data[offset..offset + 4].copy_from_slice(&0u32.to_le_bytes());

            let next = offset + rec_len;
            // Only coalesce with the successor if it lives in the *same*
            // filesystem block — directory entries must never span a block
            // boundary, so merging across one would corrupt the directory.
            let same_block = offset / block_size == next / block_size;
            if same_block && next + DIR_ENTRY_HEADER_SIZE <= data.len() {
                let next_ino = u32::from_le_bytes([
                    data[next],
                    data[next + 1],
                    data[next + 2],
                    data[next + 3],
                ]);
                let next_rec =
                    u16::from_le_bytes([data[next + 4], data[next + 5]]) as usize;

                if next_ino == 0
                    && next_rec >= DIR_ENTRY_HEADER_SIZE
                    && next + next_rec <= data.len()
                    // The merged entry must also stay within the block.
                    && offset / block_size == (next + next_rec - 1) / block_size
                {
                    let combined = rec_len + next_rec;
                    data[offset + 4..offset + 6]
                        .copy_from_slice(&(combined as u16).to_le_bytes());
                }
            }

            let block_start = (offset / block_size) * block_size;
            inode.write_at(block_start, &data[block_start..block_start + block_size])?;
            return Ok(());
        }

        offset += rec_len;
    }

    Err(FsError::NotFound)
}
