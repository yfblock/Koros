//! User process management.
//!
//! Loads ELF binaries, sets up user address space, and enters user mode.
//! Each process has:
//! - A page table (user space mapping)
//! - A kernel stack (for trap handling)
//! - A user stack
//! - A brk heap

pub mod elf;
pub mod init;
