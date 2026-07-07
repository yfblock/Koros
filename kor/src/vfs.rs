//! Core Virtual File System trait definitions.
//!
//! Concrete filesystems (ext2, ramfs, ...) in `kor-fs` implement the
//! [`SuperBlock`] and [`INode`] traits defined here.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

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

/// Metadata associated with an [`INode`].
#[derive(Clone, Debug)]
pub struct Metadata {
    pub size: usize,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub file_type: FileType,
}

/// Static capacity information reported by a mounted filesystem.
#[derive(Clone, Debug)]
pub struct FsInfo {
    pub total_blocks: usize,
    pub free_blocks: usize,
    pub total_inodes: usize,
    pub free_inodes: usize,
    pub block_size: usize,
}

/// Errors that VFS operations may return.
#[derive(Debug)]
pub enum FsError {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    NotDirectory,
    IsDirectory,
    InvalidInput,
    IoError,
    NoSpace,
    NotEmpty,
    Unsupported,
    ReadOnly,
    TooManyLinks,
    CrossDevice,
}

/// A mounted filesystem instance.
pub trait SuperBlock: Send + Sync {
    fn root_inode(&self) -> Arc<dyn INode>;
    fn sync(&self);
    fn info(&self) -> FsInfo;
    fn on_mount(&self) {}
    fn on_unmount(&self) {}
}

/// A single file, directory, or special file within a filesystem.
pub trait INode: Send + Sync {
    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError>;
    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize, FsError>;
    fn lookup(&self, name: &str) -> Result<Arc<dyn INode>, FsError>;
    fn create(&self, name: &str, file_type: FileType, mode: u32) -> Result<Arc<dyn INode>, FsError>;
    fn unlink(&self, name: &str) -> Result<(), FsError>;
    fn mkdir(&self, name: &str, mode: u32) -> Result<Arc<dyn INode>, FsError>;
    fn rmdir(&self, name: &str) -> Result<(), FsError>;
    fn getattr(&self) -> Result<Metadata, FsError>;
    fn setattr(&self, metadata: &Metadata) -> Result<(), FsError>;
    fn list(&self) -> Result<Vec<(String, u32)>, FsError>;
    fn ino(&self) -> u32;
    fn as_any(&self) -> &dyn Any;

    fn readlink(&self) -> Result<String, FsError> { Err(FsError::Unsupported) }
    fn symlink(&self, _name: &str, _target: &str) -> Result<Arc<dyn INode>, FsError> { Err(FsError::Unsupported) }
    fn link(&self, _name: &str, _target: &Arc<dyn INode>) -> Result<(), FsError> { Err(FsError::Unsupported) }
    fn rename(&self, _old_name: &str, _new_parent: &Arc<dyn INode>, _new_name: &str) -> Result<(), FsError> { Err(FsError::Unsupported) }
    fn truncate(&self, _size: usize) -> Result<(), FsError> { Err(FsError::Unsupported) }
    fn mknod(&self, _name: &str, _file_type: FileType, _mode: u32, _rdev: u32) -> Result<Arc<dyn INode>, FsError> { Err(FsError::Unsupported) }
    fn getxattr(&self, _name: &str) -> Result<Vec<u8>, FsError> { Err(FsError::Unsupported) }
    fn setxattr(&self, _name: &str, _value: &[u8]) -> Result<(), FsError> { Err(FsError::Unsupported) }
    fn listxattr(&self) -> Result<Vec<String>, FsError> { Ok(Vec::new()) }
    fn removexattr(&self, _name: &str) -> Result<(), FsError> { Err(FsError::Unsupported) }
}
