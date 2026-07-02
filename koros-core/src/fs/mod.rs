//! Core Virtual File System (VFS) trait definitions.
//!
//! This module provides the foundational abstractions for all filesystem
//! implementations in Koros.  Concrete filesystems (RamFS, ext2, …) implement
//! the [`SuperBlock`] and [`INode`] traits defined here.

pub mod ext2;
pub mod fd;
pub mod mount;
pub mod path;
pub mod ramfs;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

// ---------------------------------------------------------------------------
// File type enumeration
// ---------------------------------------------------------------------------

/// Classification of a filesystem entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FileType {
    Regular,
    Directory,
    Symlink,
    BlockDevice,
    CharDevice,
    Fifo,
    Socket,
}

// ---------------------------------------------------------------------------
// On-disk / in-memory metadata
// ---------------------------------------------------------------------------

/// Metadata associated with an [`INode`].
#[derive(Clone, Debug)]
pub struct Metadata {
    /// File size in bytes.
    pub size: usize,
    /// POSIX-style permission and type bits.
    pub mode: u32,
    /// Owner user ID.
    pub uid: u32,
    /// Owner group ID.
    pub gid: u32,
    /// Last access time (seconds since epoch).
    pub atime: u64,
    /// Last modification time (seconds since epoch).
    pub mtime: u64,
    /// Creation / status-change time (seconds since epoch).
    pub ctime: u64,
    /// What kind of file this is.
    pub file_type: FileType,
}

// ---------------------------------------------------------------------------
// Filesystem-wide information
// ---------------------------------------------------------------------------

/// Static capacity information reported by a mounted filesystem.
#[derive(Clone, Debug)]
pub struct FsInfo {
    /// Total number of blocks.
    pub total_blocks: usize,
    /// Number of free (available) blocks.
    pub free_blocks: usize,
    /// Total number of inodes.
    pub total_inodes: usize,
    /// Number of free (available) inodes.
    pub free_inodes: usize,
    /// Block size in bytes.
    pub block_size: usize,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that VFS operations may return.
#[derive(Debug)]
pub enum FsError {
    /// The requested name does not exist.
    NotFound,
    /// The caller lacks permission for the operation.
    PermissionDenied,
    /// A file or directory with that name already exists.
    AlreadyExists,
    /// Expected a directory but found something else.
    NotDirectory,
    /// The target is a directory where a non-directory was expected.
    IsDirectory,
    /// One or more arguments are invalid.
    InvalidInput,
    /// An underlying I/O error occurred.
    IoError,
    /// No space left on the filesystem.
    NoSpace,
    /// The directory is not empty (when it must be).
    NotEmpty,
    /// The operation is not supported by this filesystem.
    Unsupported,
    /// The filesystem (or this operation on it) is read-only.
    ReadOnly,
    /// Too many levels of symbolic links were encountered.
    TooManyLinks,
    /// A rename/link crosses filesystem boundaries.
    CrossDevice,
}

// ---------------------------------------------------------------------------
// SuperBlock — per-filesystem instance
// ---------------------------------------------------------------------------

/// A mounted filesystem instance.
///
/// Implementations hold whatever state is needed to manage the filesystem
/// (block device handle, caches, allocation bitmaps, …).
pub trait SuperBlock: Send + Sync {
    /// Return the root [`INode`] of this filesystem.
    fn root_inode(&self) -> Arc<dyn INode>;

    /// Flush all dirty data to the backing store.
    fn sync(&self);

    /// Report filesystem capacity and usage.
    fn info(&self) -> FsInfo;

    /// Called when the filesystem is mounted.  Implementations may record the
    /// "mounted / not cleanly unmounted" state on disk.  Default: no-op.
    fn on_mount(&self) {}

    /// Called just before the filesystem is unmounted.  Implementations
    /// should flush and mark the on-disk state clean.  Default: no-op.
    fn on_unmount(&self) {}
}

// ---------------------------------------------------------------------------
// INode — per-file / per-directory object
// ---------------------------------------------------------------------------

/// A single file, directory, or special file within a filesystem.
///
/// All implementations **must** be `Send + Sync` so inodes can be shared
/// across threads and stored in the VFS cache.
pub trait INode: Send + Sync {
    /// Read up to `buf.len()` bytes starting at `offset`.
    ///
    /// Returns the number of bytes actually read.
    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError>;

    /// Write up to `buf.len()` bytes starting at `offset`.
    ///
    /// Returns the number of bytes actually written.
    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize, FsError>;

    /// Look up a child entry by name (directory only).
    fn lookup(&self, name: &str) -> Result<Arc<dyn INode>, FsError>;

    /// Create a new child entry with the given name and type.
    ///
    /// Returns the newly created [`INode`].
    fn create(
        &self,
        name: &str,
        file_type: FileType,
        mode: u32,
    ) -> Result<Arc<dyn INode>, FsError>;

    /// Remove a non-directory child entry by name.
    fn unlink(&self, name: &str) -> Result<(), FsError>;

    /// Create a subdirectory with the given name and mode.
    ///
    /// Returns the newly created directory [`INode`].
    fn mkdir(&self, name: &str, mode: u32) -> Result<Arc<dyn INode>, FsError>;

    /// Remove an empty subdirectory by name.
    fn rmdir(&self, name: &str) -> Result<(), FsError>;

    /// Retrieve the metadata for this inode.
    fn getattr(&self) -> Result<Metadata, FsError>;

    /// Update mutable fields of this inode's metadata.
    fn setattr(&self, metadata: &Metadata) -> Result<(), FsError>;

    /// List the direct children of a directory.
    ///
    /// Returns a vector of `(name, inode_number)` pairs.
    fn list(&self) -> Result<Vec<(String, u32)>, FsError>;

    /// The inode number that uniquely identifies this object within its
    /// filesystem.
    fn ino(&self) -> u32;

    /// Upcast to [`Any`] so callers can downcast to the concrete inode type
    /// (needed by same-filesystem operations such as `link` and `rename`).
    fn as_any(&self) -> &dyn Any;

    // -- Optional operations (default: unsupported) ------------------------

    /// Read the target of a symbolic link.
    fn readlink(&self) -> Result<String, FsError> {
        Err(FsError::Unsupported)
    }

    /// Create a symbolic link named `name` pointing at `target`.
    fn symlink(&self, _name: &str, _target: &str) -> Result<Arc<dyn INode>, FsError> {
        Err(FsError::Unsupported)
    }

    /// Create a hard link named `name` in this directory referring to the
    /// existing inode `target`.
    fn link(&self, _name: &str, _target: &Arc<dyn INode>) -> Result<(), FsError> {
        Err(FsError::Unsupported)
    }

    /// Rename/move the entry `old_name` in this directory to `new_name` in
    /// `new_parent` (which may be the same directory).
    fn rename(
        &self,
        _old_name: &str,
        _new_parent: &Arc<dyn INode>,
        _new_name: &str,
    ) -> Result<(), FsError> {
        Err(FsError::Unsupported)
    }

    /// Truncate (or extend) a regular file to `size` bytes.
    fn truncate(&self, _size: usize) -> Result<(), FsError> {
        Err(FsError::Unsupported)
    }

    /// Create a device / FIFO / socket special file.
    ///
    /// `rdev` packs the device major/minor number for block/char devices and
    /// is ignored for FIFOs and sockets.
    fn mknod(
        &self,
        _name: &str,
        _file_type: FileType,
        _mode: u32,
        _rdev: u32,
    ) -> Result<Arc<dyn INode>, FsError> {
        Err(FsError::Unsupported)
    }

    // -- Extended attributes -----------------------------------------------

    /// Read the value of extended attribute `name` (e.g. `"user.foo"`).
    fn getxattr(&self, _name: &str) -> Result<Vec<u8>, FsError> {
        Err(FsError::Unsupported)
    }

    /// Set (create or replace) extended attribute `name` to `value`.
    fn setxattr(&self, _name: &str, _value: &[u8]) -> Result<(), FsError> {
        Err(FsError::Unsupported)
    }

    /// List the names of all extended attributes on this inode.
    fn listxattr(&self) -> Result<Vec<String>, FsError> {
        Ok(Vec::new())
    }

    /// Remove extended attribute `name`.
    fn removexattr(&self, _name: &str) -> Result<(), FsError> {
        Err(FsError::Unsupported)
    }
}
