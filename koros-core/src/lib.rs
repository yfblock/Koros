#![no_std]
// The assembly uses explicit .intel_syntax / .att_syntax directives because
// .altmacro conflicts with AT&T %-prefix, and multiboot.S transitions between
// .code32 and .code64 sections with different syntax requirements.
#![allow(bad_asm_style)]

extern crate alloc;

pub mod arch;
pub mod boot;
#[cfg(any(target_arch = "riscv64", target_arch = "x86_64"))]
pub mod bench;
pub mod cmdline;
pub mod drivers;
pub mod fs;
pub mod irq;
pub mod mm;
pub mod platform;
pub mod sched;
pub mod smp;
pub mod time;
pub mod trap;

use core::panic::PanicInfo;

unsafe extern "C" {
    /// The kernel entry point, provided by the `koros` binary crate.
    ///
    /// The architecture boot code (`arch::<arch>::boot::rust_entry`) calls
    /// this after early setup.
    pub fn kernel_main() -> !;
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("");
    println!("!!! KERNEL PANIC !!!");
    println!("  {}", info.message());
    if let Some(loc) = info.location() {
        println!("  at {}:{}", loc.file(), loc.line());
    }
    loop {
        core::hint::spin_loop();
    }
}

/// Run the ext2-on-virtio functional test (kernel self-check).
///
/// Uses the first block device registered by the driver-probing step; the
/// binary crate must have called the driver model (`drivers::driver::probe_fdt`
/// or the PCI probe) before this.
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "loongarch64", target_arch = "x86_64"))]
pub fn ext2_test() {
    let device = match drivers::block::first() {
        Some(dev) => dev,
        None => {
            println!("ext2 test SKIPPED: no block device registered");
            return;
        }
    };

    ext2_test_with_device(device);
}

#[cfg(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "loongarch64", target_arch = "x86_64"))]
fn ext2_test_with_device(device: alloc::sync::Arc<dyn drivers::block::BlockDevice>) {
    use fs::ext2::Ext2Fs;
    use fs::{mount, SuperBlock as SuperBlockTrait};

    // --- Open and mount the ext2 filesystem at "/" ------------------------
    let fs = match Ext2Fs::open(device.clone()) {
        Ok(fs) => fs,
        Err(e) => {
            println!("ext2 test FAILED: open: {:?}", e);
            return;
        }
    };

    let info = fs.info();
    println!(
        "ext2: {} blocks ({} free), {} inodes ({} free), block_size={}, ro={}",
        info.total_blocks,
        info.free_blocks,
        info.total_inodes,
        info.free_inodes,
        info.block_size,
        fs.is_read_only(),
    );

    if let Err(e) = mount::mount("/", fs.clone()) {
        println!("ext2 test FAILED: mount: {:?}", e);
        return;
    }

    // Root inode via the VFS, then run the functional test.
    let root = fs.root_inode();
    if let Err(e) = run_ext2_checks(&root) {
        println!("ext2 test FAILED: {:?}", e);
        return;
    }

    // Cleanly unmount: flushes all metadata/data and marks the on-disk
    // state "cleanly unmounted".
    if let Err(e) = mount::unmount("/") {
        println!("ext2 test FAILED: unmount: {:?}", e);
        return;
    }
    println!("ext2 test passed");
}

/// Exercise the read/write, symlink, hard-link, rename and truncate paths.
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "loongarch64", target_arch = "x86_64"))]
fn run_ext2_checks(root: &alloc::sync::Arc<dyn fs::INode>) -> Result<(), fs::FsError> {
    use fs::{path::resolve_path, FileType, FsError};

    // 0. Interop probe: if a Linux-authored fixture "victim" exists, delete
    //    it. This exercises unlink of a foreign inode plus xattr-block
    //    release; on a freshly-mkfs'd image it is simply absent and skipped.
    if root.lookup("victim").is_ok() {
        root.unlink("victim")?;
        println!("ext2: interop unlink(victim) OK");
    }

    // 1. mkdir + create + write + read-back.
    let dir = root.mkdir("test_dir", 0o755)?;
    let file = dir.create("test.txt", FileType::Regular, 0o644)?;
    let data = b"Hello, ext2!";
    if file.write_at(0, data)? != data.len() {
        return Err(FsError::IoError);
    }
    let mut buf = alloc::vec![0u8; data.len()];
    file.read_at(0, &mut buf)?;
    if buf != data {
        return Err(FsError::IoError);
    }
    println!("ext2: read/write OK");

    // 2. Symbolic link + readlink + path resolution following it.
    dir.symlink("link.txt", "test.txt")?;
    let link = dir.lookup("link.txt")?;
    if link.readlink()? != "test.txt" {
        return Err(FsError::IoError);
    }
    // Resolving through the symlink must land on the file's inode (the
    // relative target "test.txt" resolves within test_dir/).
    let via_link = resolve_path(root.clone(), "test_dir/link.txt")?;
    if via_link.ino() != file.ino() {
        return Err(FsError::IoError);
    }
    println!("ext2: symlink OK ({}, followed to ino={})", link.readlink()?, via_link.ino());

    // 3. Hard link — same inode reachable under a second name.
    dir.link("hard.txt", &file)?;
    let hard = dir.lookup("hard.txt")?;
    if hard.ino() != file.ino() {
        return Err(FsError::IoError);
    }
    println!("ext2: hard link OK (ino={})", hard.ino());

    // 4. Truncate the file and confirm the new size.
    file.truncate(4)?;
    if file.getattr()?.size != 4 {
        return Err(FsError::IoError);
    }
    println!("ext2: truncate OK (size={})", file.getattr()?.size);

    // 5. Rename within the directory.
    dir.rename("test.txt", &dir, "renamed.txt")?;
    if dir.lookup("test.txt").is_ok() || dir.lookup("renamed.txt").is_err() {
        return Err(FsError::IoError);
    }
    println!("ext2: rename OK");

    // 6. Path resolution through the tree.
    let resolved = resolve_path(root.clone(), "test_dir/renamed.txt")?;
    if resolved.ino() != file.ino() {
        return Err(FsError::IoError);
    }
    println!("ext2: path resolve OK");

    // 6b. Large file spanning single + double indirect blocks — validates
    //     the incremental i_blocks accounting (fsck pass 1 checks it).
    let big = dir.create("big.bin", FileType::Regular, 0o644)?;
    let chunk = alloc::vec![0xABu8; 4096];
    let total = 400 * 1024; // > 268 KiB → reaches the double-indirect region
    let mut off = 0;
    while off < total {
        if big.write_at(off, &chunk)? != chunk.len() {
            return Err(FsError::IoError);
        }
        off += chunk.len();
    }
    let mut probe = [0u8; 16];
    big.read_at(350 * 1024, &mut probe)?; // read back in the double-indirect range
    if probe.iter().any(|&b| b != 0xAB) {
        return Err(FsError::IoError);
    }
    println!("ext2: large file (indirect) OK, size={}", big.getattr()?.size);

    // 6c. Extended attributes: set / get / list / remove.
    file.setxattr("user.author", b"koros")?;
    file.setxattr("user.kind", b"regular-file")?;
    if file.getxattr("user.author")? != b"koros" {
        return Err(FsError::IoError);
    }
    let names = file.listxattr()?;
    if !names.iter().any(|n| n == "user.author") || !names.iter().any(|n| n == "user.kind") {
        return Err(FsError::IoError);
    }
    file.removexattr("user.author")?;
    if file.getxattr("user.author").is_ok() {
        return Err(FsError::IoError);
    }
    println!("ext2: xattr set/get/list/remove OK ({} left)", file.listxattr()?.len());

    // 6d. Read a Linux-authored xattr if the interop fixture is present.
    if let Ok(x) = root.lookup("xhost") {
        let v = x.getxattr("user.fromlinux")?;
        println!(
            "ext2: read Linux xattr user.fromlinux = {:?}",
            core::str::from_utf8(&v).unwrap_or("<binary>")
        );
    }

    // 7. Renaming a directory into its own descendant must be rejected
    //    (would create a cycle).
    let sub = dir.mkdir("sub", 0o755)?;
    match root.rename("test_dir", &sub, "loop") {
        Err(FsError::InvalidInput) => println!("ext2: rename-loop rejected OK"),
        other => {
            println!("ext2: rename-loop NOT rejected: {:?}", other);
            return Err(FsError::IoError);
        }
    }

    // 8. mknod a char device with a large minor to exercise the 32-bit
    //    device-number encoding (major 10, minor 1024).
    let rdev = (10u32 << 20) | 1024;
    dir.mknod("dev0", FileType::CharDevice, 0o644, rdev)?;
    println!("ext2: mknod OK");

    Ok(())
}
