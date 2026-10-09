//! aarch64 trap *mechanism*: frame layout and user-context operations.
//! Vector assembly comes from [`super::trap_macros`]; dispatch policy and
//! `init` live in the application.
//!
//! Sets `VBAR_EL1` to a hand-written vector table in assembly that
//! saves registers, calls the Rust handler, then restores and `eret`.



/// Trap frame laid out by the vector assembly (see
/// `trap_macros::trap_kernel_vec_asm!`); the application dispatch function
/// receives `&mut TrapFrame`.
#[repr(C)]
pub struct TrapFrame {
    /// x0-x30 (x8 = syscall nr, x0-x5 = args).
    pub regs: [usize; 31],
    /// SP_EL0.
    pub sp: usize,
    /// ELR_EL1.
    pub elr: usize,
    /// SPSR_EL1.
    pub spsr: usize,
    /// TPIDR_EL0.
    pub tpidr: usize,
}

const _: () = assert!(core::mem::size_of::<TrapFrame>() == 280);
const _: () = assert!(core::mem::align_of::<TrapFrame>() == core::mem::align_of::<usize>());

const USER_PSTATE_MASK: usize = 0xf000_0000;
const INSTRUCTION_ALIGN: usize = 4;
const STACK_ALIGN: usize = 16;

/// Capture the integer user context represented by `frame`.
///
/// # Safety
/// `frame` must point to a live [`TrapFrame`].
pub unsafe fn capture_user_context(frame: *mut u8) -> Option<crate::UserTrapContext> {
    let frame = unsafe { (frame as *const TrapFrame).as_ref()? };
    let mut regs = [0usize; 32];
    regs[..31].copy_from_slice(&frame.regs);
    Some(crate::UserTrapContext {
        regs,
        pc: frame.elr,
        status: frame.spsr & USER_PSTATE_MASK,
        sp: frame.sp,
        tls: frame.tpidr,
        fp_callee: [0; 12],
        fcsr: 0,
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
    if context.pc % INSTRUCTION_ALIGN != 0 || context.sp % STACK_ALIGN != 0 {
        return false;
    }
    frame.regs.copy_from_slice(&context.regs[..31]);
    frame.sp = context.sp;
    frame.elr = context.pc;
    frame.spsr = context.status & USER_PSTATE_MASK;
    frame.tpidr = context.tls;
    true
}

/// Set AArch64 signal handler arguments, link register, PC, and stack.
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
    if handler % INSTRUCTION_ALIGN != 0
        || restorer % INSTRUCTION_ALIGN != 0
        || signal_frame % STACK_ALIGN != 0
    {
        return false;
    }
    frame.regs[0] = signal;
    frame.regs[1] = siginfo;
    frame.regs[2] = ucontext;
    frame.regs[30] = restorer;
    frame.sp = signal_frame;
    frame.elr = handler;
    true
}

/// Resume a detached AArch64 user context.
///
/// # Safety
/// The matching user address space must be active.
pub unsafe fn resume_user_context(context: &crate::UserTrapContext) -> ! {
    unsafe extern "C" {
        fn aarch64_resume_user(context: *const crate::UserTrapContext) -> !;
    }
    unsafe { aarch64_resume_user(context) }
}

// Detached user resume: enter the context in `x0` directly via eret.
core::arch::global_asm!(
    r####"// Resume an owned UserTrapContext. x0 points to the context.
.global aarch64_resume_user
aarch64_resume_user:
    mov     x30, x0
    ldr     x9, [x30, #(32 * 8)]
    msr     elr_el1, x9
    mov     x9, xzr
    msr     spsr_el1, x9
    ldr     x9, [x30, #(34 * 8)]
    msr     sp_el0, x9
    ldr     x9, [x30, #(35 * 8)]
    msr     tpidr_el0, x9
    msr     daifset, #2
    ldp     x0,  x1,  [x30, #(0 * 8)]
    ldp     x2,  x3,  [x30, #(2 * 8)]
    ldp     x4,  x5,  [x30, #(4 * 8)]
    ldp     x6,  x7,  [x30, #(6 * 8)]
    ldp     x8,  x9,  [x30, #(8 * 8)]
    ldp     x10, x11, [x30, #(10 * 8)]
    ldp     x12, x13, [x30, #(12 * 8)]
    ldp     x14, x15, [x30, #(14 * 8)]
    ldp     x16, x17, [x30, #(16 * 8)]
    ldp     x18, x19, [x30, #(18 * 8)]
    ldp     x20, x21, [x30, #(20 * 8)]
    ldp     x22, x23, [x30, #(22 * 8)]
    ldp     x24, x25, [x30, #(24 * 8)]
    ldp     x26, x27, [x30, #(26 * 8)]
    ldp     x28, x29, [x30, #(28 * 8)]
    ldr     x30,      [x30, #(30 * 8)]
    eret
"####
);
