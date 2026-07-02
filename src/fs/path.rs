//! Path parsing and resolution for the VFS layer.
//!
//! Provides a [`Path`] type for decomposing filesystem paths into components,
//! and functions to resolve a path string to an [`INode`] by walking the
//! directory tree.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use super::{FileType, FsError, INode};

/// Maximum number of symbolic links that may be traversed while resolving a
/// single path before giving up (matches Linux's `MAXSYMLINKS`).
pub const MAX_SYMLINK_DEPTH: usize = 40;

// ---------------------------------------------------------------------------
// Path component iterator
// ---------------------------------------------------------------------------

/// An iterator over the non-empty, canonical components of a path string.
///
/// Consecutive slashes are collapsed, `.` components are skipped, and `..`
/// components must be handled by the caller (they depend on the directory
/// context).  Trailing slashes are ignored.
///
/// Created by [`Path::components`].
pub struct Components<'a> {
    remaining: &'a str,
}

impl<'a> Iterator for Components<'a> {
    /// Each yielded item is a single path component (e.g. `"foo"`, `".."`).
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        // Skip leading/consecutive slashes.
        self.remaining = self.remaining.trim_start_matches('/');

        if self.remaining.is_empty() {
            return None;
        }

        // Find the end of the current component.
        let end = self.remaining.find('/').unwrap_or(self.remaining.len());
        let component = &self.remaining[..end];
        self.remaining = &self.remaining[end..];

        Some(component)
    }
}

// ---------------------------------------------------------------------------
// Path
// ---------------------------------------------------------------------------

/// A parsed filesystem path.
///
/// Wraps an owned path string and provides helpers for iterating over its
/// components and querying whether the path is absolute.
///
/// # Examples
///
/// ```ignore
/// let p = Path::new("/usr/bin/ls");
/// assert!(p.is_absolute());
/// assert_eq!(p.components().collect::<Vec<_>>(), vec!["usr", "bin", "ls"]);
/// ```
pub struct Path {
    inner: String,
}

impl Path {
    /// Create a new `Path` from the given string.
    pub fn new(s: &str) -> Self {
        Self {
            inner: s.to_string(),
        }
    }

    /// Returns `true` if the path starts with `/`.
    pub fn is_absolute(&self) -> bool {
        self.inner.starts_with('/')
    }

    /// Returns an iterator over the path's components.
    ///
    /// Consecutive slashes are collapsed and trailing slashes are ignored.
    /// The special components `.` and `..` are yielded as-is; the caller
    /// is responsible for interpreting them in directory context.
    pub fn components(&self) -> Components<'_> {
        Components {
            remaining: &self.inner,
        }
    }
}

// ---------------------------------------------------------------------------
// Resolution helpers
// ---------------------------------------------------------------------------

/// Resolve a path string starting from `start_inode`, returning the
/// corresponding [`INode`].
///
/// * If `path` starts with `/`, resolution restarts at `start_inode` (which
///   the caller must pass as the filesystem root).
/// * Otherwise resolution begins at `start_inode`.
///
/// The special components `.` and `..` are handled during the walk, and
/// **symbolic links are followed** (including a terminal symlink) up to
/// [`MAX_SYMLINK_DEPTH`] levels.  A relative link target is resolved against
/// the directory that contains the link; an absolute target restarts at the
/// root.
///
/// # Errors
///
/// * [`FsError::NotFound`] — a component does not exist.
/// * [`FsError::NotDirectory`] — a non-terminal component is not a directory.
/// * [`FsError::TooManyLinks`] — more than [`MAX_SYMLINK_DEPTH`] symlinks were
///   encountered (a likely loop).
pub fn resolve_path(
    start_inode: Arc<dyn INode>,
    path: &str,
) -> Result<Arc<dyn INode>, FsError> {
    let root = start_inode.clone();
    let mut depth = 0;
    resolve_inner(&root, start_inode, path, &mut depth)
}

/// Core resolver, threading the filesystem `root` (for absolute symlink
/// targets) and a shared symlink-`depth` counter through recursive symlink
/// expansion.
fn resolve_inner(
    root: &Arc<dyn INode>,
    start: Arc<dyn INode>,
    path: &str,
    depth: &mut usize,
) -> Result<Arc<dyn INode>, FsError> {
    let parsed = Path::new(path);
    let mut current = if parsed.is_absolute() {
        root.clone()
    } else {
        start
    };

    // Collect components so we can tell which one is terminal.
    let components: Vec<&str> = parsed.components().collect();

    for name in components {
        match name {
            "." => {}
            ".." => {
                current = lookup_entry(&current, "..")?;
            }
            _ => {
                let next = lookup_entry(&current, name)?;
                // Follow symbolic links, resolving relative targets against
                // the directory that contains the link (`current`).
                if is_symlink(&next)? {
                    *depth += 1;
                    if *depth > MAX_SYMLINK_DEPTH {
                        return Err(FsError::TooManyLinks);
                    }
                    let target = next.readlink()?;
                    current = resolve_inner(root, current.clone(), &target, depth)?;
                } else {
                    current = next;
                }
            }
        }
    }

    Ok(current)
}

/// Return `true` if `inode` is a symbolic link.
fn is_symlink(inode: &Arc<dyn INode>) -> Result<bool, FsError> {
    Ok(inode.getattr()?.file_type == FileType::Symlink)
}

/// Look up a single child entry `name` inside `dir_inode`.
///
/// This is a thin wrapper around [`INode::lookup`] that provides a
/// consistent error context.
///
/// # Errors
///
/// * [`FsError::NotDirectory`] if `dir_inode` is not a directory.
/// * [`FsError::NotFound`] if `name` does not exist.
pub fn lookup_entry(
    dir_inode: &Arc<dyn INode>,
    name: &str,
) -> Result<Arc<dyn INode>, FsError> {
    dir_inode.lookup(name)
}
