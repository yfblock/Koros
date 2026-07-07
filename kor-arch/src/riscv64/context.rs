//! riscv64 task context switch (callee-saved registers).

use core::arch::naked_asm;

/// Saved state for a cooperative context switch: return address, stack
/// pointer, and the callee-saved registers `s0`–`s11`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TaskContext {
    ra: usize,
    sp: usize,
    s: [usize; 12],
}

impl TaskContext {
    pub const fn zero() -> Self {
        Self { ra: 0, sp: 0, s: [0; 12] }
    }

    /// Prepare a fresh context so the first switch into it starts executing at
    /// `entry` on `stack_top`.
    pub fn init(&mut self, entry: usize, stack_top: usize) {
        self.ra = entry;
        self.sp = stack_top;
        self.s = [0; 12];
    }
}

/// Save the current callee-saved state into `*prev` and restore `*next`.
/// Returns into `next`'s saved `ra`.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn context_switch(prev: *mut TaskContext, next: *const TaskContext) {
    // a0 = prev, a1 = next
    naked_asm!(
        "sd  ra,  0*8(a0)",
        "sd  sp,  1*8(a0)",
        "sd  s0,  2*8(a0)",
        "sd  s1,  3*8(a0)",
        "sd  s2,  4*8(a0)",
        "sd  s3,  5*8(a0)",
        "sd  s4,  6*8(a0)",
        "sd  s5,  7*8(a0)",
        "sd  s6,  8*8(a0)",
        "sd  s7,  9*8(a0)",
        "sd  s8,  10*8(a0)",
        "sd  s9,  11*8(a0)",
        "sd  s10, 12*8(a0)",
        "sd  s11, 13*8(a0)",
        "ld  ra,  0*8(a1)",
        "ld  sp,  1*8(a1)",
        "ld  s0,  2*8(a1)",
        "ld  s1,  3*8(a1)",
        "ld  s2,  4*8(a1)",
        "ld  s3,  5*8(a1)",
        "ld  s4,  6*8(a1)",
        "ld  s5,  7*8(a1)",
        "ld  s6,  8*8(a1)",
        "ld  s7,  9*8(a1)",
        "ld  s8,  10*8(a1)",
        "ld  s9,  11*8(a1)",
        "ld  s10, 12*8(a1)",
        "ld  s11, 13*8(a1)",
        "ret",
    )
}
