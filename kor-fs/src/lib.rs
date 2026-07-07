#![no_std]
//! Virtual File System infrastructure: mount table, path resolution, file
//! descriptors, the ext2 and ramfs implementations, and the block cache.
//!
//! VFS trait definitions (`INode`, `SuperBlock`, `FsError`, ...) live in
//! [`kor`] and are re-exported here so the `super::`-relative imports of the
//! moved files resolve unchanged.

extern crate alloc;

pub mod block_cache;
pub mod ext2;
pub mod fd;
pub mod mount;
pub mod path;
pub mod ramfs;

// Re-export the VFS / block types from `kor` at the crate root so the moved
// files' `super::`-relative and `crate::`-absolute imports resolve.
pub use kor::{BlockDevice, BlockError, FileType, FsError, FsInfo, INode, Metadata, SuperBlock};
