//! Init process spawning from the mounted root filesystem.

use alloc::string::String;

use super::elf;
use crate::fs;
use crate::mm::AddressSpace;

/// Spawn the first user process.
pub fn spawn_init() {
    let executable = if fs::exists("/bin/busybox") {
        "/bin/busybox"
    } else {
        "/bin/sh"
    };
    kor::println!("[korix] loading {}...", executable);
    let image = match fs::read_file(executable) {
        Ok(data) => data,
        Err(error) => {
            kor::println!("[korix] cannot read {}: {:?}", executable, error);
            return;
        }
    };
    kor::println!("[korix] userspace image: {} bytes", image.len());
    if !elf::is_elf(&image) {
        kor::println!("[korix] {} is not a valid ELF", executable);
        return;
    }
    let interpreter_path = match elf::interpreter_path(&image) {
        Ok(path) => path,
        Err(error) => {
            kor::println!("[korix] cannot parse interpreter: {}", error);
            return;
        }
    };
    let interpreter = match interpreter_path.as_deref() {
        Some(path) => match fs::read_file(path) {
            Ok(data) => Some(data),
            Err(error) => {
                kor::println!("[korix] cannot read interpreter {}: {:?}", path, error);
                return;
            }
        },
        None => None,
    };

    let mut address_space = match AddressSpace::empty() {
        Ok(address_space) => address_space,
        Err(error) => {
            kor::println!("[korix] cannot create init address space: {:?}", error);
            return;
        }
    };
    let argv = [String::from("/bin/sh")];
    match elf::load_program(&mut address_space, &image, interpreter.as_deref(), &argv) {
        Ok(prog) => {
            kor::println!(
                "[korix] loaded: entry={:#x}, sp={:#x}",
                prog.entry,
                prog.stack_top
            );

            address_space.activate();
            if let Some(process) = crate::sched::current_process() {
                let mut process = process.lock();
                process.executable = alloc::string::String::from("/bin/busybox");
                process.name = *b"busybox\0\0\0\0\0\0\0\0\0";
                process.replace_address_space(address_space);
            } else {
                core::mem::forget(address_space);
            }
            kor::println!("[korix] entering user mode...");
            kor_arch::enter_user(prog.entry, prog.stack_top);
        }
        Err(e) => {
            kor::println!("[korix] ELF load failed: {}", e);
        }
    }
}
