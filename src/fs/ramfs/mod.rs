//! In-memory filesystem (RamFS) implementation.
//!
//! Provides a simple in-memory filesystem backed by `Vec<u8>` for file content
//! and `BTreeMap` for directory entries.  Intended as a reference implementation
//! and test harness for the VFS trait layer.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::any::Any;
use core::sync::atomic::{AtomicU32, Ordering};
use spin::Mutex;

use super::{FileType, FsError, FsInfo, INode, Metadata, SuperBlock};

// ---------------------------------------------------------------------------
// Inode number allocator
// ---------------------------------------------------------------------------

/// Global monotonic inode number counter.
static NEXT_INO: AtomicU32 = AtomicU32::new(1);

/// Allocate a fresh inode number.
fn alloc_ino() -> u32 {
    NEXT_INO.fetch_add(1, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// RamINode
// ---------------------------------------------------------------------------

/// An in-memory inode.
///
/// Directories populate [`children`]; regular files populate [`content`].
/// Symlinks and special files are not supported by this implementation.
struct RamINode {
    /// Inode number (unique per filesystem instance).
    ino: u32,
    /// What kind of entry this is.
    file_type: FileType,
    /// Byte content for regular files (empty for directories).
    content: Mutex<Vec<u8>>,
    /// Child entries for directories (empty for regular files).
    children: Mutex<BTreeMap<String, Arc<RamINode>>>,
    /// Cached metadata.
    metadata: Mutex<Metadata>,
    /// Weak self-reference so trait methods can return `Arc<dyn INode>`.
    self_ref: Mutex<Option<Weak<RamINode>>>,
    /// Parent directory (None for the root).
    parent: Mutex<Option<Weak<RamINode>>>,
}

impl RamINode {
    /// Create a new directory inode.
    fn new_dir(parent: Option<Weak<RamINode>>, mode: u32) -> Arc<Self> {
        let node = Arc::new(Self {
            ino: alloc_ino(),
            file_type: FileType::Directory,
            content: Mutex::new(Vec::new()),
            children: Mutex::new(BTreeMap::new()),
            metadata: Mutex::new(Metadata {
                size: 0,
                mode,
                uid: 0,
                gid: 0,
                atime: 0,
                mtime: 0,
                ctime: 0,
                file_type: FileType::Directory,
            }),
            self_ref: Mutex::new(None),
            parent: Mutex::new(parent),
        });
        *node.self_ref.lock() = Some(Arc::downgrade(&node));
        node
    }

    /// Create a new regular-file inode.
    fn new_file(parent: Weak<RamINode>, mode: u32) -> Arc<Self> {
        let node = Arc::new(Self {
            ino: alloc_ino(),
            file_type: FileType::Regular,
            content: Mutex::new(Vec::new()),
            children: Mutex::new(BTreeMap::new()),
            metadata: Mutex::new(Metadata {
                size: 0,
                mode,
                uid: 0,
                gid: 0,
                atime: 0,
                mtime: 0,
                ctime: 0,
                file_type: FileType::Regular,
            }),
            self_ref: Mutex::new(None),
            parent: Mutex::new(Some(parent)),
        });
        *node.self_ref.lock() = Some(Arc::downgrade(&node));
        node
    }

    /// Upgrade the stored self-reference to an `Arc<dyn INode>`.
    fn self_inode(&self) -> Result<Arc<dyn INode>, FsError> {
        self.self_ref
            .lock()
            .as_ref()
            .and_then(Weak::upgrade)
            .map(|arc| arc as Arc<dyn INode>)
            .ok_or(FsError::IoError)
    }
}

// ---------------------------------------------------------------------------
// INode trait implementation
// ---------------------------------------------------------------------------

impl INode for RamINode {
    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
        if self.file_type != FileType::Regular {
            return Err(FsError::InvalidInput);
        }
        let content = self.content.lock();
        if offset >= content.len() {
            return Ok(0);
        }
        let end = (offset + buf.len()).min(content.len());
        let len = end - offset;
        buf[..len].copy_from_slice(&content[offset..end]);
        Ok(len)
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize, FsError> {
        if self.file_type != FileType::Regular {
            return Err(FsError::InvalidInput);
        }
        let new_size = {
            let mut content = self.content.lock();
            let end = offset + buf.len();
            if end > content.len() {
                content.resize(end, 0);
            }
            content[offset..end].copy_from_slice(buf);
            content.len()
        };
        self.metadata.lock().size = new_size;
        Ok(buf.len())
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn INode>, FsError> {
        if self.file_type != FileType::Directory {
            return Err(FsError::NotDirectory);
        }
        match name {
            "." => self.self_inode(),
            ".." => {
                let guard = self.parent.lock();
                match guard.as_ref() {
                    Some(weak) => weak
                        .upgrade()
                        .map(|arc| arc as Arc<dyn INode>)
                        .ok_or(FsError::IoError),
                    None => self.self_inode(), // root → parent is self
                }
            }
            _ => self
                .children
                .lock()
                .get(name)
                .map(|arc| Arc::clone(arc) as Arc<dyn INode>)
                .ok_or(FsError::NotFound),
        }
    }

    fn create(
        &self,
        name: &str,
        file_type: FileType,
        mode: u32,
    ) -> Result<Arc<dyn INode>, FsError> {
        if self.file_type != FileType::Directory {
            return Err(FsError::NotDirectory);
        }
        let self_weak = self
            .self_ref
            .lock()
            .clone()
            .ok_or(FsError::IoError)?;

        let mut children = self.children.lock();
        if children.contains_key(name) {
            return Err(FsError::AlreadyExists);
        }

        let child: Arc<RamINode> = match file_type {
            FileType::Regular => RamINode::new_file(self_weak, mode),
            FileType::Directory => RamINode::new_dir(Some(self_weak), mode),
            _ => return Err(FsError::InvalidInput),
        };

        children.insert(name.to_string(), Arc::clone(&child));
        Ok(child as Arc<dyn INode>)
    }

    fn unlink(&self, name: &str) -> Result<(), FsError> {
        if self.file_type != FileType::Directory {
            return Err(FsError::NotDirectory);
        }
        let mut children = self.children.lock();
        let child = children.get(name).ok_or(FsError::NotFound)?;
        if child.file_type == FileType::Directory {
            return Err(FsError::IsDirectory);
        }
        children.remove(name);
        Ok(())
    }

    fn mkdir(&self, name: &str, mode: u32) -> Result<Arc<dyn INode>, FsError> {
        if self.file_type != FileType::Directory {
            return Err(FsError::NotDirectory);
        }
        let self_weak = self
            .self_ref
            .lock()
            .clone()
            .ok_or(FsError::IoError)?;

        let mut children = self.children.lock();
        if children.contains_key(name) {
            return Err(FsError::AlreadyExists);
        }

        let child = RamINode::new_dir(Some(self_weak), mode);
        children.insert(name.to_string(), Arc::clone(&child));
        Ok(child as Arc<dyn INode>)
    }

    fn rmdir(&self, name: &str) -> Result<(), FsError> {
        if self.file_type != FileType::Directory {
            return Err(FsError::NotDirectory);
        }
        let mut children = self.children.lock();
        let child = children.get(name).ok_or(FsError::NotFound)?;
        if child.file_type != FileType::Directory {
            return Err(FsError::NotDirectory);
        }
        if !child.children.lock().is_empty() {
            return Err(FsError::NotEmpty);
        }
        children.remove(name);
        Ok(())
    }

    fn getattr(&self) -> Result<Metadata, FsError> {
        Ok(self.metadata.lock().clone())
    }

    fn setattr(&self, metadata: &Metadata) -> Result<(), FsError> {
        *self.metadata.lock() = metadata.clone();
        Ok(())
    }

    fn list(&self) -> Result<Vec<(String, u32)>, FsError> {
        if self.file_type != FileType::Directory {
            return Err(FsError::NotDirectory);
        }
        let children = self.children.lock();
        Ok(children
            .iter()
            .map(|(name, node)| (name.clone(), node.ino))
            .collect())
    }

    fn ino(&self) -> u32 {
        self.ino
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// RamFs — the SuperBlock
// ---------------------------------------------------------------------------

/// An in-memory filesystem instance.
///
/// Holds a reference to the root directory inode.  All metadata lives in
/// memory; [`sync`](SuperBlock::sync) is a no-op.
pub struct RamFs {
    /// Root directory of this filesystem.
    root: Arc<RamINode>,
}

impl RamFs {
    /// Create a new RamFS with an empty root directory (mode `0o755`).
    pub fn new() -> Self {
        Self {
            root: RamINode::new_dir(None, 0o755),
        }
    }
}

impl Default for RamFs {
    fn default() -> Self {
        Self::new()
    }
}

impl SuperBlock for RamFs {
    fn root_inode(&self) -> Arc<dyn INode> {
        Arc::clone(&self.root) as Arc<dyn INode>
    }

    fn sync(&self) {
        // No-op: everything is in memory.
    }

    fn info(&self) -> FsInfo {
        FsInfo {
            total_blocks: 0,
            free_blocks: 0,
            total_inodes: NEXT_INO.load(Ordering::Relaxed) as usize,
            free_inodes: 0,
            block_size: 4096,
        }
    }
}
