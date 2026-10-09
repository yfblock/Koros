//! riscv64 trap handling.
use core::arch::{asm, global_asm};
use kor::println;

#[repr(C)]
struct TrapFrame {
    regs: [usize; 32],
    sstatus: usize,
    sepc: usize,
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

global_asm!(include_str!("trap.S"), tf_size = const core::mem::size_of::<TrapFrame>());

unsafe extern "Rust" {
    fn kernelvec();
}

#[unsafe(no_mangle)]
extern "C" fn handle_trap(tf: &mut TrapFrame) {
    let scause: usize;
    let stval: usize;
    unsafe {
        asm!("csrr {}, scause", out(reg) scause);
        asm!("csrr {}, stval", out(reg) stval);
    }
    let is_interrupt = (scause >> 63) != 0;
    let code = scause & ((1 << 63) - 1);

    if is_interrupt && code == 5 {
        kor::on_timer();
        return;
    }
    if is_interrupt && code == 9 {
        crate::dispatch_external();
        return;
    }

    if !is_interrupt && code == 8 {
        if let Some(handler) = kor::trap_callbacks::syscall_handler() {
            unsafe {
                kor::trap_callbacks::user_trap_enter(tf as *mut TrapFrame as *mut u8);
            }
            let nr = tf.regs[17];
            let args = [
                tf.regs[10],
                tf.regs[11],
                tf.regs[12],
                tf.regs[13],
                tf.regs[14],
                tf.regs[15],
            ];
            let ret = handler(nr, &args);
            tf.regs[10] = ret as usize;
            // PC-advance policy (asm-generic numbers, native to riscv64):
            // rt_sigreturn restores its own saved PC, and a successful
            // execve installs a brand-new context — neither may bump sepc.
            const NR_RT_SIGRETURN: usize = 139;
            const NR_EXECVE: usize = 221;
            if nr != NR_RT_SIGRETURN && (nr != NR_EXECVE || ret < 0) {
                tf.sepc += 4;
            }
            unsafe {
                kor::trap_callbacks::user_trap_leave(tf as *mut TrapFrame as *mut u8);
            }
            return;
        }
    }
    println!("");
    println!("=== TRAP ===");
    println!(
        "scause  = {:#x} ({} #{})",
        scause,
        if is_interrupt {
            "Interrupt"
        } else {
            "Exception"
        },
        code
    );
    println!("stval   = {:#x}", stval);
    println!("sepc    = {:#x}", tf.sepc);
    println!("sstatus = {:#x}", tf.sstatus);
    panic!("Unhandled trap: scause={:#x}, sepc={:#x}", scause, tf.sepc);
}

pub fn init() {
    unsafe {
        asm!("csrw stvec, {}", in(reg) kernelvec as *const () as usize);
    }
    println!(
        "trap: stvec set to kernelvec ({:#x})",
        kernelvec as *const () as usize
    );
}
