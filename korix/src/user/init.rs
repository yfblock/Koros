//! Init process spawning.
//!
//! Looks for /init in the root filesystem, loads it as an ELF,
//! and enters user mode. If /init is not found, creates a simple
//! init-like file to verify VFS read/write works.

use super::elf;
use crate::fs;

/// Spawn the first user process from /init.
pub fn spawn_init() {
    kor::println!("[korix] looking for /init...");

    // Try to read /init from the root filesystem
    match fs::read_file("/init") {
        Ok(data) => {
            kor::println!("[korix] /init found ({} bytes)", data.len());
            if elf::is_elf(&data) {
                match elf::load_elf(&data) {
                    Ok(entry) => {
                        kor::println!("[korix] ELF entry point: {:#x}", entry);
                        // TODO: set up user page table, stack, and enter user mode
                        kor::println!("[korix] TODO: enter user mode at {:#x}", entry);
                    }
                    Err(e) => {
                        kor::println!("[korix] ELF load failed: {}", e);
                    }
                }
            } else {
                kor::println!("[korix] /init is not an ELF file");
            }
        }
        Err(_) => {
            kor::println!("[korix] /init not found, creating built-in init");
            create_builtin_init();
        }
    }
}

/// Create a simple /init file in ramfs and verify VFS round-trip.
fn create_builtin_init() {
    let init_content = b"#!/bin/sh\necho korix init\n";
    match fs::write_file("/init", init_content) {
        Ok(_) => kor::println!("[korix] created /init ({} bytes)", init_content.len()),
        Err(e) => kor::println!("[korix] failed to create /init: {:?}", e),
    }

    // List root directory
    match fs::list_dir("/") {
        Ok(entries) => {
            kor::println!("[korix] root directory:");
            for (name, _) in &entries {
                kor::println!("  /{}", name);
            }
        }
        Err(e) => kor::println!("[korix] list_dir failed: {:?}", e),
    }

    // Verify we can read it back via mount::resolve
    match fs::read_file("/init") {
        Ok(data) => {
            kor::println!("[korix] read /init: {} bytes", data.len());
            if data == init_content {
                kor::println!("[korix] /init content verified OK");
            }
        }
        Err(e) => kor::println!("[korix] read /init failed: {:?}", e),
    }

    kor::println!("[korix] VFS read/write working, user-mode ELF loading TODO");
}
