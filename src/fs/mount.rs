//! Mount table for the VFS layer.
//!
//! Tracks which [`SuperBlock`] instances are mounted at which paths, and
//! provides lookup by longest-prefix match.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use spin::Mutex;

use super::{FsError, INode, SuperBlock};

// ---------------------------------------------------------------------------
// MountPoint
// ---------------------------------------------------------------------------

/// A single entry in the mount table.
///
/// Associates a mount path (e.g. `"/mnt/usb"`) with a filesystem instance.
#[derive(Clone)]
pub struct MountPoint {
    /// The absolute path where this filesystem is mounted.
    pub path: String,
    /// The filesystem instance mounted at `path`.
    pub fs: Arc<dyn SuperBlock>,
}

// ---------------------------------------------------------------------------
// MountTable
// ---------------------------------------------------------------------------

/// A table of all currently mounted filesystems.
///
/// Mount paths are stored in a [`BTreeMap`] keyed by their canonical string
/// form.  Lookup uses **longest-prefix matching**: given a path like
/// `"/mnt/usb/docs/readme.txt"`, the table returns the filesystem mounted at
/// the longest matching prefix (e.g. `"/mnt/usb"` over `"/mnt"`).
pub struct MountTable {
    /// Maps mount-point path → filesystem instance.
    table: BTreeMap<String, Arc<dyn SuperBlock>>,
}

impl MountTable {
    /// Create an empty mount table.
    pub fn new() -> Self {
        Self {
            table: BTreeMap::new(),
        }
    }

    /// Mount `fs` at `path`.
    ///
    /// `path` is normalized: trailing slashes are stripped and a leading `/`
    /// is ensured.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::AlreadyExists`] if a filesystem is already mounted
    /// at exactly `path`.
    pub fn mount(&mut self, path: &str, fs: Arc<dyn SuperBlock>) -> Result<(), FsError> {
        let canonical = canonicalize_mount_path(path);

        if self.table.contains_key(&canonical) {
            return Err(FsError::AlreadyExists);
        }

        self.table.insert(canonical, fs);
        Ok(())
    }

    /// Unmount the filesystem at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::NotFound`] if no filesystem is mounted at `path`.
    pub fn unmount(&mut self, path: &str) -> Result<(), FsError> {
        let canonical = canonicalize_mount_path(path);

        if self.table.remove(&canonical).is_some() {
            Ok(())
        } else {
            Err(FsError::NotFound)
        }
    }

    /// Find the filesystem whose mount point is the **longest prefix** of
    /// `path`.
    ///
    /// Returns `None` if no mount point matches (the table is empty or no
    /// prefix matches).
    pub fn resolve_mount(&self, path: &str) -> Option<Arc<dyn SuperBlock>> {
        let canonical = canonicalize_mount_path(path);

        // BTreeMap iterates in sorted order.  We find the last entry whose
        // key is a prefix of `canonical`.
        self.table
            .range(..=canonical.clone())
            .rfind(|(mount_path, _)| {
                canonical == *mount_path.as_str()
                    || canonical.starts_with(&format!("{mount_path}/"))
            })
            .map(|(_, fs)| Arc::clone(fs))
    }

    /// Return the mount-point path (key) that is the longest prefix of
    /// `path`, if any.
    pub fn longest_prefix(&self, path: &str) -> Option<String> {
        let canonical = canonicalize_mount_path(path);
        self.table
            .range(..=canonical.clone())
            .rfind(|(mount_path, _)| {
                canonical == *mount_path.as_str()
                    || canonical.starts_with(&format!("{mount_path}/"))
            })
            .map(|(mount_path, _)| mount_path.clone())
    }
}

impl Default for MountTable {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Global mount table
// ---------------------------------------------------------------------------

/// The kernel-wide mount table.
static MOUNTS: Mutex<MountTable> = Mutex::new(MountTable {
    table: BTreeMap::new(),
});

/// Mount `fs` at the absolute path `path` in the global mount table.
pub fn mount(path: &str, fs: Arc<dyn SuperBlock>) -> Result<(), FsError> {
    MOUNTS.lock().mount(path, Arc::clone(&fs))?;
    // Notify the filesystem so it can mark itself "mounted" on disk.
    fs.on_mount();
    Ok(())
}

/// Unmount whatever is mounted at `path`.
pub fn unmount(path: &str) -> Result<(), FsError> {
    let fs = MOUNTS.lock().resolve_mount(path).ok_or(FsError::NotFound)?;
    // Flush and mark clean before detaching.
    fs.on_unmount();
    MOUNTS.lock().unmount(path)
}

/// Resolve `path` to the inode it names, walking into the filesystem mounted
/// at its longest matching prefix.
pub fn resolve(path: &str) -> Result<Arc<dyn INode>, FsError> {
    let fs = MOUNTS.lock().resolve_mount(path).ok_or(FsError::NotFound)?;
    let mount_path = MOUNTS
        .lock()
        .longest_prefix(path)
        .unwrap_or_else(|| "/".to_string());

    // Strip the mount-point prefix so the remainder is relative to the
    // filesystem root.
    let canonical = canonicalize_mount_path(path);
    let relative = canonical
        .strip_prefix(&mount_path)
        .unwrap_or(&canonical)
        .trim_start_matches('/');

    super::path::resolve_path(fs.root_inode(), relative)
}

/// Flush every mounted filesystem to its backing store.
pub fn sync_all() {
    for fs in MOUNTS.lock().table.values() {
        fs.sync();
    }
}

/// Normalize a mount path: strip trailing slashes and ensure a leading `/`.
fn canonicalize_mount_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');

    if trimmed.is_empty() {
        // The root "/" is the only path that reduces to "/".
        return "/".to_string();
    }

    if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        alloc::format!("/{trimmed}")
    }
}
