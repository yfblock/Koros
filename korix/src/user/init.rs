//! Init process spawning.
//!
//! Loads the busybox binary from /bin/busybox in the rootfs,
//! sets up the user address space, and enters user mode as /bin/sh.

use super::elf;
use crate::arch::user_ctx;
use crate::fs;

/// Spawn the first user process.
pub fn spawn_init() {
    kor::println!("[korix] loading /bin/busybox...");

    // Read busybox from the root filesystem.
    let busybox = match fs::read_file("/bin/busybox") {
        Ok(data) => data,
        Err(e) => {
            kor::println!("[korix] cannot read /bin/busybox: {:?}", e);
            return;
        }
    };
    kor::println!("[korix] busybox: {} bytes", busybox.len());

    if !elf::is_elf(&busybox) {
        kor::println!("[korix] /bin/busybox is not a valid ELF!");
        return;
    }

    match elf::load_elf(&busybox) {
        Ok(prog) => {
            kor::println!(
                "[korix] loaded: entry={:#x}, sp={:#x}",
                prog.entry, prog.stack_top
            );

            let ctx = user_ctx::UserContext::new(prog.entry, prog.stack_top);
            kor::println!("[korix] entering user mode...");
            unsafe {
                user_ctx::enter_user(&ctx);
            }
        }
        Err(e) => {
            kor::println!("[korix] ELF load failed: {}", e);
        }
    }
}
