//! Filesystem initialization and VFS helpers.
//!
//! Mounts the root filesystem (ramfs), and provides helpers to:
//! - Read a file by path into a Vec<u8>
//! - Create files and write content
//! - List directories

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use kor::{FsError, INode, FileType};
use kor_fs::mount;

/// Mount the root filesystem.
pub fn init_rootfs() {
    let mounts = &crate::MOUNTS;
    match kor_fs::registry::mount_named(mounts, "ramfs", None, "/") {
        Ok(_) => kor::println!("[korix] mounted ramfs on /"),
        Err(e) => kor::println!("[korix] ramfs mount failed: {:?}", e),
    }
}

/// Resolve a path to an INode via the mount table.
pub fn resolve(path: &str) -> Result<Arc<dyn INode>, FsError> {
    mount::resolve(&crate::MOUNTS, path)
}

/// Read a file by path into a Vec<u8>.
pub fn read_file(path: &str) -> Result<Vec<u8>, FsError> {
    let inode = resolve(path)?;
    let meta = inode.getattr()?;
    let mut buf = alloc::vec![0u8; meta.size];
    let n = inode.read_at(0, &mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// Create a file at the given path with the given content.
pub fn write_file(path: &str, content: &[u8]) -> Result<(), FsError> {
    // Split path into parent directory and filename
    let (parent_path, filename) = match path.rfind('/') {
        Some(0) => ("/", &path[1..]),
        Some(idx) => (&path[..idx], &path[idx+1..]),
        None => ("/", path),
    };
    let parent = resolve(parent_path)?;
    let file = parent.create(filename, FileType::Regular, 0o644)?;
    file.write_at(0, content)?;
    Ok(())
}

/// List directory entries at the given path.
pub fn list_dir(path: &str) -> Result<Vec<(String, u32)>, FsError> {
    let inode = resolve(path)?;
    inode.list()
}

/// Check if a file exists at the given path.
pub fn exists(path: &str) -> bool {
    resolve(path).is_ok()
}

/// Create a directory at the given path.
pub fn mkdir(path: &str) -> Result<(), FsError> {
    let (parent_path, dirname) = match path.rfind('/') {
        Some(0) => ("/", &path[1..]),
        Some(idx) => (&path[..idx], &path[idx+1..]),
        None => ("/", path),
    };
    let parent = resolve(parent_path)?;
    parent.mkdir(dirname, 0o755)?;
    Ok(())
}
