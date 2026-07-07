//! Filesystem-driver registry.
//!
//! Concrete filesystem implementations (ext2, ramfs, ...) live in their own
//! crates and register a [`FileSystemDriver`] here at boot.  The composition
//! layer then looks up a driver by name and mounts an instance — a block-backed
//! filesystem passes `Some(device)`, a nodev filesystem passes `None`.

use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::Mutex;

use kor::{BlockDevice, FileSystemDriver, FsError, SuperBlock};

static FS_DRIVERS: Mutex<Vec<&'static dyn FileSystemDriver>> = Mutex::new(Vec::new());

/// Register a filesystem driver so it can later be mounted by name.
///
/// Drivers are registered once at boot (e.g. from `kernel_main`) and live for
/// the lifetime of the kernel, hence the `'static` bound.
pub fn register_filesystem(driver: &'static dyn FileSystemDriver) {
    FS_DRIVERS.lock().push(driver);
}

/// Look up a registered filesystem driver by its [`FileSystemDriver::name`].
pub fn find_filesystem(name: &str) -> Option<&'static dyn FileSystemDriver> {
    FS_DRIVERS
        .lock()
        .iter()
        .find(|d| d.name() == name)
        .copied()
}

/// Convenience: look up `name`, mount an instance on `device`, and mount it at
/// `path` in `mounts`.  Returns the freshly-created filesystem root.
pub fn mount_named(
    mounts: &Mutex<crate::mount::MountTable>,
    name: &str,
    device: Option<Arc<dyn BlockDevice>>,
    path: &str,
) -> Result<Arc<dyn SuperBlock>, FsError> {
    let driver = find_filesystem(name).ok_or(FsError::NotFound)?;
    let fs = driver.mount(device)?;
    crate::mount::mount(mounts, path, Arc::clone(&fs))?;
    Ok(fs)
}
