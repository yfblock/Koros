//! User-mode context management.
//!
//! Each user process has a UserContext that tracks:
//! - Saved registers (for context switch)
//! - User stack pointer
//! - User program counter
//! - Address space (page table root)
//!
//! On a context switch, the kernel:
//! 1. Saves the outgoing process's user registers
//! 2. Switches to the incoming process's page table
//! 3. Restores the incoming process's user registers
//! 4. Returns to user mode (eret / sret / iretq)

#[repr(C)]
pub struct UserContext {
    pub regs: [u64; 31],
    pub sp: u64,
    pub pc: u64,
    pub pstate: u64,
}

impl UserContext {
    pub const fn new() -> Self {
        Self {
            regs: [0; 31],
            sp: 0,
            pc: 0,
            pstate: 0,
        }
    }

    /// Set the entry point and stack for a new user process.
    pub fn init(&mut self, entry: usize, stack_top: usize) {
        self.pc = entry as u64;
        self.sp = stack_top as u64;

        #[cfg(target_arch = "riscv64")]
        {
            self.pstate = 0; // User mode, interrupts enabled
        }
        #[cfg(target_arch = "aarch64")]
        {
            // SPSR: EL0h, interrupts enabled
            self.pstate = 0x3C5;
        }
        #[cfg(target_arch = "x86_64")]
        {
            // RFLAGS: interrupts enabled
            self.pstate = 0x202;
        }
    }
}

/// Switch to user mode and run the given context.
///
/// # Safety
/// The page table must already be set up for the user process.
pub unsafe fn enter_user(_ctx: &UserContext) -> ! {
    // TODO: implement arch-specific user-mode entry
    // riscv64: sret
    // aarch64: eret
    // x86_64: iretq
    loop {
        core::hint::spin_loop();
    }
}
