//! riscv64 task context switch (callee-saved registers).

use core::arch::naked_asm;

/// Saved cooperative context: integer and floating-point callee-saved state.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TaskContext {
    ra: usize,
    sp: usize,
    s: [usize; 12],
    fs: [u64; 12],
    fcsr: usize,
}

impl TaskContext {
    pub const fn zero() -> Self {
        Self {
            ra: 0,
            sp: 0,
            s: [0; 12],
            fs: [0; 12],
            fcsr: 0,
        }
    }

    /// Prepare a fresh context so the first switch into it starts executing at
    /// `entry` on `stack_top`.
    pub fn init(&mut self, entry: usize, stack_top: usize) {
        self.ra = entry;
        self.sp = stack_top;
        self.s = [0; 12];
        self.fs = [0; 12];
        self.fcsr = 0;
    }
}

/// Save the current callee-saved state into `*prev` and restore `*next`.
/// Returns into `next`'s saved `ra`.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn context_switch(prev: *mut TaskContext, next: *const TaskContext) {
    // a0 = prev, a1 = next
    naked_asm!(
        ".option push",
        ".option arch, +d",
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
        "fsd fs0,  14*8(a0)",
        "fsd fs1,  15*8(a0)",
        "fsd fs2,  16*8(a0)",
        "fsd fs3,  17*8(a0)",
        "fsd fs4,  18*8(a0)",
        "fsd fs5,  19*8(a0)",
        "fsd fs6,  20*8(a0)",
        "fsd fs7,  21*8(a0)",
        "fsd fs8,  22*8(a0)",
        "fsd fs9,  23*8(a0)",
        "fsd fs10, 24*8(a0)",
        "fsd fs11, 25*8(a0)",
        "csrr t0, fcsr",
        "sd t0, 26*8(a0)",
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
        "fld fs0,  14*8(a1)",
        "fld fs1,  15*8(a1)",
        "fld fs2,  16*8(a1)",
        "fld fs3,  17*8(a1)",
        "fld fs4,  18*8(a1)",
        "fld fs5,  19*8(a1)",
        "fld fs6,  20*8(a1)",
        "fld fs7,  21*8(a1)",
        "fld fs8,  22*8(a1)",
        "fld fs9,  23*8(a1)",
        "fld fs10, 24*8(a1)",
        "fld fs11, 25*8(a1)",
        "ld t0, 26*8(a1)",
        "csrw fcsr, t0",
        "ret",
        ".option pop",
    )
}
