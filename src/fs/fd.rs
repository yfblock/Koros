//! File descriptor abstraction.
//!
//! A [`FileDescriptor`] wraps an [`INode`] with a seek position and open flags,
//! providing the POSIX-like `read` / `write` / `seek` / `close` interface that
//! userspace-facing code will use.

use alloc::sync::Arc;
use core::fmt;

use super::{FsError, INode};

// ---------------------------------------------------------------------------
// OpenFlags — bitflags describing how a file was opened
// ---------------------------------------------------------------------------

/// Bitflags describing how a file descriptor was opened.
///
/// These mirror the subset of POSIX `open(2)` flags that Koros supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenFlags {
    bits: u32,
}

impl OpenFlags {
    /// Open for reading.
    pub const READ: Self = Self { bits: 1 << 0 };
    /// Open for writing.
    pub const WRITE: Self = Self { bits: 1 << 1 };
    /// Append mode — writes always go to the end.
    pub const APPEND: Self = Self { bits: 1 << 2 };
    /// Create the file if it does not exist.
    pub const CREATE: Self = Self { bits: 1 << 3 };
    /// Truncate the file to zero length on open.
    pub const TRUNCATE: Self = Self { bits: 1 << 4 };

    /// Create an empty (no flags set) value.
    pub const fn empty() -> Self {
        Self { bits: 0 }
    }

    /// Return `true` if `flag` is set.
    pub const fn contains(self, flag: Self) -> bool {
        (self.bits & flag.bits) == flag.bits
    }

    /// Return `true` if no flags are set.
    pub const fn is_empty(self) -> bool {
        self.bits == 0
    }
}

impl core::ops::BitOr for OpenFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self {
            bits: self.bits | rhs.bits,
        }
    }
}

impl core::ops::BitOrAssign for OpenFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.bits |= rhs.bits;
    }
}

impl fmt::Display for OpenFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        let mut flag = |name: &str, bit: Self, f: &mut fmt::Formatter<'_>| -> fmt::Result {
            if (self.bits & bit.bits) != 0 {
                if !first {
                    f.write_str(" | ")?;
                }
                f.write_str(name)?;
                first = false;
            }
            Ok(())
        };
        flag("READ", Self::READ, f)?;
        flag("WRITE", Self::WRITE, f)?;
        flag("APPEND", Self::APPEND, f)?;
        flag("CREATE", Self::CREATE, f)?;
        flag("TRUNCATE", Self::TRUNCATE, f)?;
        if first {
            f.write_str("NONE")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SeekFrom — relative seek anchor
// ---------------------------------------------------------------------------

/// Describes the reference point for a seek operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeekFrom {
    /// Seek from the beginning of the file.
    Start(usize),
    /// Seek relative to the current position (signed).
    Current(isize),
    /// Seek relative to the end of the file (signed).
    End(isize),
}

// ---------------------------------------------------------------------------
// FileDescriptor
// ---------------------------------------------------------------------------

/// An open file handle that combines an [`INode`] with a seek position and
/// access flags.
///
/// This is the kernel-side analogue of a POSIX file descriptor — it does **not**
/// yet own an integer fd number (that will be added when the process / fd-table
/// layer is implemented).
pub struct FileDescriptor {
    /// The underlying inode.
    inode: Arc<dyn INode>,
    /// Current byte offset within the file.
    position: usize,
    /// Flags the file was opened with.
    flags: OpenFlags,
    /// POSIX mode bits (permissions, setuid, etc.).
    mode: u32,
}

impl FileDescriptor {
    /// Create a new file descriptor wrapping `inode`.
    ///
    /// `flags` records how the file was opened; `mode` stores the POSIX
    /// permission bits.
    pub fn new(inode: Arc<dyn INode>, flags: OpenFlags, mode: u32) -> Self {
        Self {
            inode,
            position: 0,
            flags,
            mode,
        }
    }

    /// Return the open flags for this descriptor.
    pub fn flags(&self) -> OpenFlags {
        self.flags
    }

    /// Return the current seek position.
    pub fn position(&self) -> usize {
        self.position
    }

    /// Return the POSIX mode bits.
    pub fn mode(&self) -> u32 {
        self.mode
    }

    // -- I/O operations ----------------------------------------------------

    /// Read up to `buf.len()` bytes from the current position.
    ///
    /// Advances the position by the number of bytes actually read.
    /// Returns the number of bytes read, or an [`FsError`].
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError> {
        let n = self.inode.read_at(self.position, buf)?;
        self.position += n;
        Ok(n)
    }

    /// Write up to `buf.len()` bytes at the current position.
    ///
    /// If the descriptor was opened with [`OpenFlags::APPEND`], the write is
    /// redirected to the end of the file first.
    /// Advances the position by the number of bytes actually written.
    /// Returns the number of bytes written, or an [`FsError`].
    pub fn write(&mut self, buf: &[u8]) -> Result<usize, FsError> {
        if self.flags.contains(OpenFlags::APPEND) {
            let meta = self.inode.getattr()?;
            self.position = meta.size;
        }
        let n = self.inode.write_at(self.position, buf)?;
        self.position += n;
        Ok(n)
    }

    /// Seek to a new position within the file.
    ///
    /// Returns the new absolute position, or an [`FsError`] if the resulting
    /// offset would be negative.
    pub fn seek(&mut self, whence: SeekFrom) -> Result<usize, FsError> {
        match whence {
            SeekFrom::Start(pos) => {
                self.position = pos;
            }
            SeekFrom::Current(delta) => {
                if delta >= 0 {
                    self.position += delta as usize;
                } else {
                    let abs = (-delta) as usize;
                    if abs > self.position {
                        return Err(FsError::InvalidInput);
                    }
                    self.position -= abs;
                }
            }
            SeekFrom::End(delta) => {
                let size = self.inode.getattr()?.size;
                if delta >= 0 {
                    self.position = size + delta as usize;
                } else {
                    let abs = (-delta) as usize;
                    if abs > size {
                        return Err(FsError::InvalidInput);
                    }
                    self.position = size - abs;
                }
            }
        }
        Ok(self.position)
    }

    /// Close the file descriptor.
    ///
    /// Currently a no-op.  In the future this will flush dirty pages and
    /// synchronise with the backing store.
    pub fn close(self) -> Result<(), FsError> {
        // Future: flush, sync, release locks, etc.
        Ok(())
    }
}
