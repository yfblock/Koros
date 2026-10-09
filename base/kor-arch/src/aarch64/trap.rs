//! aarch64 trap handling.
//!
//! Sets `VBAR_EL1` to a hand-written vector table in assembly that
//! saves registers, calls the Rust handler, then restores and `eret`.

use core::arch::{asm, global_asm};
use kor::println;

#[repr(C)]
struct TrapFrame {
    /// x0 – x30
    regs: [usize; 31],
    /// SP_EL0
    sp: usize,
    /// ELR_EL1
    elr: usize,
    /// SPSR_EL1
    spsr: usize,
    /// TPIDR_EL0
    tpidr: usize,
}

const _: () = assert!(core::mem::size_of::<TrapFrame>() == 280);
const _: () = assert!(core::mem::align_of::<TrapFrame>() == core::mem::align_of::<usize>());

const USER_PSTATE_MASK: usize = 0xf000_0000;
const INSTRUCTION_ALIGN: usize = 4;
const STACK_ALIGN: usize = 16;
const USER_SEGFAULT_STATUS: i32 = 128 + 11;

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

global_asm!(include_str!("trap.S"));

unsafe extern "C" {
    fn exception_vector_base();
}

/// Called from assembly with `x0 = &mut TrapFrame, x1 = kind, x2 = source`.
#[unsafe(no_mangle)]
extern "C" fn handle_trap(tf: &mut TrapFrame, kind: u64, source: u64) {
    // IRQ (kind 1): dispatch through the interrupt controller.
    if kind == 1 {
        crate::dispatch_external();
        return;
    }

    // Synchronous exception
    if kind == 0 {
        let esr: u64;
        let far: u64;
        unsafe {
            asm!("mrs {}, esr_el1", out(reg) esr);
            asm!("mrs {}, far_el1", out(reg) far);
        }
        let ec = esr >> 26;

        // SVC from AArch64 (EC=0x15) — syscall
        if ec == 0x15 && source == 2 {
            if let Some(handler) = kor::trap_callbacks::syscall_handler() {
                unsafe {
                    kor::trap_callbacks::user_trap_enter(tf as *mut TrapFrame as *mut u8);
                }
                let nr = tf.regs[8];
                let args = [
                    tf.regs[0], tf.regs[1], tf.regs[2], tf.regs[3], tf.regs[4], tf.regs[5],
                ];
                let ret = handler(nr, &args);
                tf.regs[0] = ret as usize;
                unsafe {
                    kor::trap_callbacks::user_trap_leave(tf as *mut TrapFrame as *mut u8);
                }
                return;
            }
        }

        if source == 2 {
            kor::trap_callbacks::user_exit(USER_SEGFAULT_STATUS);
        }
        println!("");
        println!("=== TRAP ===");
        println!(
            "ESR_EL1 = {:#x} (EC={:#x}, ISS={:#x})",
            esr,
            ec,
            esr & 0xFFFFFF
        );
        println!("FAR_EL1 = {:#x}", far);
        println!("ELR_EL1 = {:#x}", tf.elr);
        println!("SPSR_EL1 = {:#x}", tf.spsr);
        panic!("Unhandled trap: EC={:#x}, ELR={:#x}", ec, tf.elr);
    }

    println!("");
    println!("=== TRAP ===");
    println!("kind={} source={}", kind, source);
    panic!("Unhandled trap");
}

/// Install the trap handler: set `VBAR_EL1` and enable FPSIMD.
pub fn init() {
    unsafe {
        // Enable FPSIMD/NEON access for EL0 and EL1 (FPEN=0b11)
        asm!("msr cpacr_el1, {0}", in(reg) 0x300000u64);
        asm!("isb");
        asm!("msr vbar_el1, {0}", in(reg) exception_vector_base as *const () as u64);
    }
    println!(
        "trap: VBAR_EL1 set to {:#x}",
        exception_vector_base as *const () as usize
    );
}
