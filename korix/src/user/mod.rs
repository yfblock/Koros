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

/// A user process.
pub struct Process {
    pub pid: usize,
    pub ctx: crate::arch::user_ctx::UserContext,
    // TODO: page table root, kernel stack, user stack, brk
}

impl Process {
    pub fn new() -> Self {
        Self {
            pid: 0,
            ctx: crate::arch::user_ctx::UserContext::new(),
        }
    }
}
