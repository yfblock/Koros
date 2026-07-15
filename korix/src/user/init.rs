//! Init process spawning.
//!
//! Loads the busybox binary from /bin/busybox in the rootfs,
//! sets up the user address space, and enters user mode as /bin/sh.

use alloc::string::String;

use super::elf;
use crate::arch::user_ctx;
use crate::fs;
use crate::mm::AddressSpace;

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

    let mut address_space = match AddressSpace::empty() {
        Ok(address_space) => address_space,
        Err(error) => {
            kor::println!("[korix] cannot create init address space: {:?}", error);
            return;
        }
    };
    let argv = [String::from("/bin/sh")];
    match elf::load_elf(&mut address_space, &busybox, &argv) {
        Ok(prog) => {
            kor::println!(
                "[korix] loaded: entry={:#x}, sp={:#x}",
                prog.entry,
                prog.stack_top
            );

            address_space.activate();
            if let Some(process) = crate::sched::current_process() {
                process.lock().replace_address_space(address_space);
            } else {
                core::mem::forget(address_space);
            }
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
