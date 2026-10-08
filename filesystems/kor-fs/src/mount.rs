//! Mount table for the VFS layer.
//!
//! Tracks which [`SuperBlock`] instances are mounted at which paths, and
//! provides lookup by longest-prefix match.

use alloc::collections::BTreeMap;
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
    pub const fn new() -> Self {
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
            .rfind(|(mount_path, _)| is_mount_prefix(mount_path, &canonical))
            .map(|(_, fs)| Arc::clone(fs))
    }

    /// Return the mount-point path (key) that is the longest prefix of
    /// `path`, if any.
    pub fn longest_prefix(&self, path: &str) -> Option<String> {
        let canonical = canonicalize_mount_path(path);
        self.table
            .range(..=canonical.clone())
            .rfind(|(mount_path, _)| is_mount_prefix(mount_path, &canonical))
            .map(|(mount_path, _)| mount_path.clone())
    }
}

/// Check whether `mount_path` is a prefix of `canonical` for mount-matching
/// purposes.
///
/// A mount path matches if:
/// - `canonical` equals `mount_path` exactly, or
/// - `canonical` starts with `mount_path` and the next character is `/`
///   (or `mount_path` itself ends with `/`, which only happens for root).
///
/// The previous implementation used `format!("{mount_path}/")` which
/// produced `"//"` for the root mount `/`, causing all non-root paths
/// to fail prefix matching.
fn is_mount_prefix(mount_path: &str, canonical: &str) -> bool {
    canonical == mount_path
        || canonical.starts_with(mount_path)
            && (mount_path.ends_with('/')
                || canonical.as_bytes().get(mount_path.len()) == Some(&b'/'))
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
// Free-function wrappers operating on an explicit `&Mutex<MountTable>`.
// The composition layer owns the instance (e.g. `koros::registries::MOUNTS`);
// there is no global mount table in the library.
// ---------------------------------------------------------------------------

/// Mount `fs` at the absolute path `path`.
pub fn mount(
    mounts: &Mutex<MountTable>,
    path: &str,
    fs: Arc<dyn SuperBlock>,
) -> Result<(), FsError> {
    mounts.lock().mount(path, Arc::clone(&fs))?;
    // Roll back the table entry if marking the filesystem mounted fails.
    if let Err(error) = fs.on_mount() {
        let _ = mounts.lock().unmount(path);
        return Err(error);
    }
    Ok(())
}

/// Unmount whatever is mounted at `path`.
pub fn unmount(mounts: &Mutex<MountTable>, path: &str) -> Result<(), FsError> {
    let canonical = canonicalize_mount_path(path);
    let fs = mounts
        .lock()
        .table
        .get(&canonical)
        .cloned()
        .ok_or(FsError::NotFound)?;
    // Keep the filesystem attached if flushing or marking it clean fails.
    fs.on_unmount()?;
    mounts.lock().unmount(path)
}

/// Resolve `path` to the inode it names, walking into the filesystem mounted
/// at its longest matching prefix.
pub fn resolve(mounts: &Mutex<MountTable>, path: &str) -> Result<Arc<dyn INode>, FsError> {
    let (fs, mount_path) = {
        let g = mounts.lock();
        let fs = g.resolve_mount(path).ok_or(FsError::NotFound)?;
        let mount_path = g.longest_prefix(path).unwrap_or_else(|| "/".to_string());
        (fs, mount_path)
    };
    let canonical = canonicalize_mount_path(path);
    let relative = canonical
        .strip_prefix(&mount_path)
        .unwrap_or(&canonical)
        .trim_start_matches('/');
    super::path::resolve_path(fs.root_inode(), relative)
}

/// Flush every mounted filesystem to its backing store.
pub fn sync_all(mounts: &Mutex<MountTable>) -> Result<(), FsError> {
    for fs in mounts.lock().table.values() {
        fs.sync()?;
    }
    Ok(())
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
