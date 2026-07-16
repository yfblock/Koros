//! Filesystem initialization and VFS helpers.
//!
//! Mounts the root filesystem (ramfs), and provides helpers to:
//! - Read a file by path into a Vec<u8>
//! - Create files and write content
//! - List directories

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use kor::{FileType, FsError, INode};
use kor_fs::mount;

/// Mount the first ext2 block device as root, falling back to embedded ramfs.
pub fn init_rootfs() {
    let mounts = &crate::MOUNTS;
    let mounted_ext2 = crate::registries::BLOCKS
        .first()
        .and_then(|device| kor_ext2::Ext2Fs::open(device).ok())
        .and_then(|fs| mounts.lock().mount("/", fs).ok())
        .is_some();
    if mounted_ext2 {
        if resolve("/bin/busybox").is_ok() || resolve("/bin/sh").is_ok() {
            kor::println!("[korix] mounted ext2 root filesystem");
            return;
        }
        kor::println!("[korix] ext2 root has no /bin/sh; using ramfs root");
        let _ = mounts.lock().unmount("/");
    }
    match kor_fs::registry::mount_named(mounts, "ramfs", None, "/") {
        Ok(_) => kor::println!("[korix] mounted ramfs on /"),
        Err(error) => {
            kor::println!("[korix] ramfs mount failed: {:?}", error);
            return;
        }
    }
    #[cfg(target_arch = "riscv64")]
    let embedded_busybox: &[u8] = include_bytes!("../../busybox-riscv64");
    #[cfg(target_arch = "aarch64")]
    let embedded_busybox: &[u8] = include_bytes!("../../busybox-aarch64");
    #[cfg(target_arch = "x86_64")]
    let embedded_busybox: &[u8] = include_bytes!("../../busybox-x86_64");

    // Create standard directories
    for dir in &["/bin", "/dev", "/proc", "/etc", "/tmp"] {
        let _ = mkdir(dir);
    }

    // Write busybox binary to /bin/busybox.
    let busybox = embedded_busybox;
    match write_file("/bin/busybox", busybox) {
        Ok(_) => kor::println!("[korix] /bin/busybox ({} bytes)", busybox.len()),
        Err(e) => kor::println!("[korix] failed to write /bin/busybox: {:?}", e),
    }

    // Create symlinks for common applets
    let applets = [
        "sh", "ls", "echo", "cat", "date", "true", "false", "pwd", "mkdir", "rmdir", "rm", "cp",
        "mv", "ln", "head", "tail", "wc", "sort", "uniq", "grep", "sed", "awk", "vi", "env", "id",
        "whoami", "hostname", "sleep", "test", "[",
    ];
    for applet in &applets {
        if let Ok(root) = resolve("/bin") {
            let _ = root.symlink(applet, "/bin/busybox");
        }
    }
    // /bin/[ needs special handling — create as regular name

    // Create /etc/passwd and /etc/group for ls/id/whoami
    #[cfg(not(target_arch = "x86_64"))]
    let _ = write_file("/etc/passwd", b"root:x:0:0:root:/root:/bin/sh\n");
    #[cfg(not(target_arch = "x86_64"))]
    let _ = write_file("/etc/group", b"root:x:0:\n");
    let _ = write_file("/etc/hostname", b"korix\n");
    let _ = mkdir("/root");

    kor::println!(
        "[korix] rootfs ready: /bin/busybox + {} applets",
        applets.len()
    );
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
        Some(idx) => (&path[..idx], &path[idx + 1..]),
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
        Some(idx) => (&path[..idx], &path[idx + 1..]),
        None => ("/", path),
    };
    let parent = resolve(parent_path)?;
    parent.mkdir(dirname, 0o755)?;
    Ok(())
}
