//! loongarch64 task context switch (callee-saved registers).

use core::arch::naked_asm;

/// Saved callee state: `ra`, `sp`, `fp` ($r22) and `s0`–`s8` ($r23–$r31).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TaskContext {
    ra: usize,
    sp: usize,
    fp: usize,
    s: [usize; 9],
}

impl TaskContext {
    pub const fn zero() -> Self {
        Self { ra: 0, sp: 0, fp: 0, s: [0; 9] }
    }

    /// Prepare a fresh context so the first switch starts at `entry` on
    /// `stack_top`.
    pub fn init(&mut self, entry: usize, stack_top: usize) {
        self.ra = entry;
        self.sp = stack_top;
        self.fp = 0;
        self.s = [0; 9];
    }
}

/// Save the current callee state into `*prev` and restore `*next`; returns
/// into `next`'s saved `ra`.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn context_switch(prev: *mut TaskContext, next: *const TaskContext) {
    // $a0 = prev, $a1 = next
    naked_asm!(
        "st.d $ra,  $a0, 0*8",
        "st.d $sp,  $a0, 1*8",
        "st.d $fp,  $a0, 2*8",
        "st.d $r23, $a0, 3*8",
        "st.d $r24, $a0, 4*8",
        "st.d $r25, $a0, 5*8",
        "st.d $r26, $a0, 6*8",
        "st.d $r27, $a0, 7*8",
        "st.d $r28, $a0, 8*8",
        "st.d $r29, $a0, 9*8",
        "st.d $r30, $a0, 10*8",
        "st.d $r31, $a0, 11*8",
        "ld.d $ra,  $a1, 0*8",
        "ld.d $sp,  $a1, 1*8",
        "ld.d $fp,  $a1, 2*8",
        "ld.d $r23, $a1, 3*8",
        "ld.d $r24, $a1, 4*8",
        "ld.d $r25, $a1, 5*8",
        "ld.d $r26, $a1, 6*8",
        "ld.d $r27, $a1, 7*8",
        "ld.d $r28, $a1, 8*8",
        "ld.d $r29, $a1, 9*8",
        "ld.d $r30, $a1, 10*8",
        "ld.d $r31, $a1, 11*8",
        "jr $ra",
    )
}
