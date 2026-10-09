//! riscv64 trap *mechanism*: frame layout, per-hart user scratch, and the
//! user-context capture/restore/signal/resume operations.  Vector assembly
//! comes from [`super::trap_macros`] and the dispatch policy lives in the
//! application.

use core::arch::asm;

/// Kernel/user trap frame laid out by the vector assembly (see
/// `trap_macros::trap_kernel_vec_asm!`); the application dispatch function
/// receives `&mut TrapFrame`.
#[repr(C)]
pub struct TrapFrame {
    /// x0-x31; index 0 unused, 2 = sp, 4 = tp, 10-17 = a0-a7, 17 = a7(syscall nr).
    pub regs: [usize; 32],
    /// sstatus at trap entry.
    pub sstatus: usize,
    /// sepc at trap entry.
    pub sepc: usize,
}

const SSTATUS_SPP: usize = 1 << 8;
const SSTATUS_SPIE: usize = 1 << 5;
const SSTATUS_FS: usize = 3 << 13;
const INSTRUCTION_ALIGN: usize = 2;

/// Per-hart state used to recover kernel state after user code uses `tp` for TLS.
#[repr(C, align(16))]
pub struct UserTrapScratch {
    kernel_sp: usize,
    hart_id: usize,
    user_tp: usize,
    user_sp: usize,
}

/// Assembly-visible user trap scratch slots, indexed by hart id.
#[unsafe(no_mangle)]
pub static mut RISCV_USER_TRAP_SCRATCH: [UserTrapScratch; kor::smp::MAX_CPUS] = [const {
    UserTrapScratch {
        kernel_sp: 0,
        hart_id: 0,
        user_tp: 0,
        user_sp: 0,
    }
};
    kor::smp::MAX_CPUS];
/// Prepare `sscratch` before the first transition from this kernel stack to user mode.
pub fn prepare_user_entry(kernel_sp: usize) {
    // `sscratch` changes from the kernel marker to a user-entry record here.
    // No kernel interrupt may observe that intermediate state before `sret`.
    crate::irq::disable();
    let hart = super::smp::cpu_id();
    let scratch = unsafe { &raw mut RISCV_USER_TRAP_SCRATCH[hart] };
    unsafe {
        (*scratch).kernel_sp = kernel_sp;
        (*scratch).hart_id = hart;
        asm!("csrw sscratch, {}", in(reg) scratch);
    }
}

/// Capture the integer user context represented by `frame`.
///
/// # Safety
/// `frame` must point to a live [`TrapFrame`].
pub unsafe fn capture_user_context(frame: *mut u8) -> Option<crate::UserTrapContext> {
    let frame = unsafe { (frame as *const TrapFrame).as_ref()? };
    if frame.sstatus & SSTATUS_SPP != 0 {
        return None;
    }
    let mut fp_callee = [0u64; 12];
    let fcsr: usize;
    unsafe {
        core::arch::asm!(
            "fsd fs0, 0*8({fp})", "fsd fs1, 1*8({fp})", "fsd fs2, 2*8({fp})", "fsd fs3, 3*8({fp})",
            "fsd fs4, 4*8({fp})", "fsd fs5, 5*8({fp})", "fsd fs6, 6*8({fp})", "fsd fs7, 7*8({fp})",
            "fsd fs8, 8*8({fp})", "fsd fs9, 9*8({fp})", "fsd fs10, 10*8({fp})", "fsd fs11, 11*8({fp})",
            "csrr {fcsr}, fcsr", fp = in(reg) fp_callee.as_mut_ptr(), fcsr = out(reg) fcsr,
        );
    }
    Some(crate::UserTrapContext {
        regs: frame.regs,
        pc: frame.sepc,
        status: frame.sstatus & (SSTATUS_SPIE | SSTATUS_FS),
        sp: frame.regs[2],
        tls: frame.regs[4],
        fp_callee,
        fcsr,
    })
}

/// Restore an integer user context into `frame` after privilege sanitization.
///
/// # Safety
/// `frame` must point to a live [`TrapFrame`].
pub unsafe fn restore_user_context(frame: *mut u8, context: &crate::UserTrapContext) -> bool {
    let Some(frame) = (unsafe { (frame as *mut TrapFrame).as_mut() }) else {
        return false;
    };
    if context.pc % INSTRUCTION_ALIGN != 0 || context.sp % core::mem::size_of::<usize>() != 0 {
        return false;
    }
    unsafe {
        core::arch::asm!(
            "fld fs0, 0*8({fp})", "fld fs1, 1*8({fp})", "fld fs2, 2*8({fp})", "fld fs3, 3*8({fp})",
            "fld fs4, 4*8({fp})", "fld fs5, 5*8({fp})", "fld fs6, 6*8({fp})", "fld fs7, 7*8({fp})",
            "fld fs8, 8*8({fp})", "fld fs9, 9*8({fp})", "fld fs10, 10*8({fp})", "fld fs11, 11*8({fp})",
            "csrw fcsr, {fcsr}", fp = in(reg) context.fp_callee.as_ptr(), fcsr = in(reg) context.fcsr,
        );
    }
    frame.regs = context.regs;
    frame.regs[0] = 0;
    frame.regs[2] = context.sp;
    frame.regs[4] = context.tls;
    frame.sepc = context.pc;
    frame.sstatus = (frame.sstatus & !(SSTATUS_SPP | SSTATUS_FS)) | SSTATUS_SPIE | SSTATUS_FS;
    true
}

/// Set RISC-V signal handler arguments, return address, PC, and stack.
///
/// # Safety
/// `frame` must point to a live user [`TrapFrame`].
pub unsafe fn setup_user_signal(
    frame: *mut u8,
    handler: usize,
    restorer: usize,
    signal_frame: usize,
    signal: usize,
    siginfo: usize,
    ucontext: usize,
) -> bool {
    let Some(frame) = (unsafe { (frame as *mut TrapFrame).as_mut() }) else {
        return false;
    };
    if handler % INSTRUCTION_ALIGN != 0 || restorer % INSTRUCTION_ALIGN != 0 {
        return false;
    }
    frame.regs[1] = restorer;
    frame.regs[2] = signal_frame;
    frame.regs[10] = signal;
    frame.regs[11] = siginfo;
    frame.regs[12] = ucontext;
    frame.sepc = handler;
    true
}

/// Resume a detached RISC-V user context.
///
/// # Safety
/// The matching user address space must be active.
pub unsafe fn resume_user_context(context: &crate::UserTrapContext) -> ! {
    prepare_user_entry(current_kernel_sp());
    unsafe {
        core::arch::asm!(
            "fld fs0, 0*8({fp})", "fld fs1, 1*8({fp})", "fld fs2, 2*8({fp})", "fld fs3, 3*8({fp})",
            "fld fs4, 4*8({fp})", "fld fs5, 5*8({fp})", "fld fs6, 6*8({fp})", "fld fs7, 7*8({fp})",
            "fld fs8, 8*8({fp})", "fld fs9, 9*8({fp})", "fld fs10, 10*8({fp})", "fld fs11, 11*8({fp})",
            "csrw fcsr, {fcsr}", fp = in(reg) context.fp_callee.as_ptr(), fcsr = in(reg) context.fcsr,
        );
    }
    unsafe extern "C" {
        fn riscv_resume_user(context: *const crate::UserTrapContext) -> !;
    }
    unsafe { riscv_resume_user(context) }
}

#[inline]
fn current_kernel_sp() -> usize {
    let sp: usize;
    unsafe { asm!("mv {}, sp", out(reg) sp) };
    sp
}

// Detached user resume: enter the context in `a0` directly via sret.
core::arch::global_asm!(
    r#"
.global riscv_resume_user
riscv_resume_user:
    mv      t6, a0
    ld      t0, 32 * 8(t6)
    csrw    sepc, t0
    ld      t0, 33 * 8(t6)
    li      t1, ((1 << 5) | (3 << 13))
    or      t0, t0, t1
    li      t1, (1 << 8)
    not     t1, t1
    and     t0, t0, t1
    csrw    sstatus, t0
    # `prepare_user_entry` left sscratch pointing at this hart's trap record.
    ld      ra,   1 * 8(t6)
    ld      gp,   3 * 8(t6)
    ld      tp,  35 * 8(t6)
    ld      t0,   5 * 8(t6)
    ld      t1,   6 * 8(t6)
    ld      t2,   7 * 8(t6)
    ld      s0,   8 * 8(t6)
    ld      s1,   9 * 8(t6)
    ld      a0,  10 * 8(t6)
    ld      a1,  11 * 8(t6)
    ld      a2,  12 * 8(t6)
    ld      a3,  13 * 8(t6)
    ld      a4,  14 * 8(t6)
    ld      a5,  15 * 8(t6)
    ld      a6,  16 * 8(t6)
    ld      a7,  17 * 8(t6)
    ld      s2,  18 * 8(t6)
    ld      s3,  19 * 8(t6)
    ld      s4,  20 * 8(t6)
    ld      s5,  21 * 8(t6)
    ld      s6,  22 * 8(t6)
    ld      s7,  23 * 8(t6)
    ld      s8,  24 * 8(t6)
    ld      s9,  25 * 8(t6)
    ld      s10, 26 * 8(t6)
    ld      s11, 27 * 8(t6)
    ld      t3,  28 * 8(t6)
    ld      t4,  29 * 8(t6)
    ld      t5,  30 * 8(t6)
    ld      sp,  34 * 8(t6)
    ld      t6,  31 * 8(t6)
    fence.i
    sret
"#
);
