//! aarch64 task context switch (callee-saved registers x19-x30 + sp).

use core::arch::naked_asm;

/// Saved callee state: `x19`–`x30` (x29=fp, x30=lr) and the stack pointer.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TaskContext {
    /// x19 … x30 (index 11 = x30/lr).
    regs: [usize; 12],
    sp: usize,
}

impl TaskContext {
    pub const fn zero() -> Self {
        Self { regs: [0; 12], sp: 0 }
    }

    /// Prepare a fresh context so the first switch starts at `entry` on
    /// `stack_top` (the entry address goes into the saved `lr`).
    pub fn init(&mut self, entry: usize, stack_top: usize) {
        self.regs = [0; 12];
        self.regs[11] = entry; // x30 / lr
        self.sp = stack_top;
    }
}

/// Save the current callee state into `*prev` and restore `*next`; returns
/// into `next`'s saved `lr`.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn context_switch(prev: *mut TaskContext, next: *const TaskContext) {
    // x0 = prev, x1 = next
    naked_asm!(
        "stp x19, x20, [x0, #0]",
        "stp x21, x22, [x0, #16]",
        "stp x23, x24, [x0, #32]",
        "stp x25, x26, [x0, #48]",
        "stp x27, x28, [x0, #64]",
        "stp x29, x30, [x0, #80]",
        "mov x2, sp",
        "str x2, [x0, #96]",
        "ldp x19, x20, [x1, #0]",
        "ldp x21, x22, [x1, #16]",
        "ldp x23, x24, [x1, #32]",
        "ldp x25, x26, [x1, #48]",
        "ldp x27, x28, [x1, #64]",
        "ldp x29, x30, [x1, #80]",
        "ldr x2, [x1, #96]",
        "mov sp, x2",
        "ret",
    )
}
