#![no_std]
//! Virtual File System infrastructure: mount table, path resolution, file
//! descriptors, the block cache, and the filesystem-driver registry.
//!
//! VFS trait definitions (`INode`, `SuperBlock`, `FsError`, `BlockDevice`,
//! `FileSystemDriver`, ...) live in [`kor`] and are re-exported here.
//! Concrete filesystem implementations (ext2, ramfs) live in their own crates
//! (`kor-ext2`, `kor-ramfs`) and register a [`FileSystemDriver`] at boot via
//! [`registry::register_filesystem`].

extern crate alloc;

pub mod block_cache;
pub mod fd;
pub mod mount;
pub mod path;
pub mod registry;

// Re-export the VFS / block types from `kor` at the crate root so downstream
// code can name them through `kor_fs`.
pub use kor::{
    BlockDevice, BlockError, FileSystemDriver, FileType, FsError, FsInfo, INode, Metadata,
    SuperBlock,
};
pub use registry::{find_filesystem, mount_named, register_filesystem};
