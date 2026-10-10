//! aarch64 trap handling for korix: vector composed from the kor-arch
//! fragment, dispatch implemented here — GIC-delivered IRQs and the timer
//! PPI, the SVC syscall entry, and user-fault exit via the scheduler.

use core::arch::{asm, global_asm};

use kor::println;
use kor_arch::TrapFrame;

/// `WIFSIGNALED`-style status for a user-mode fault: SIGSEGV.
const USER_SEGFAULT_STATUS: i32 = 128 + 11;

global_asm!(kor_arch::trap_kernel_vec_asm!());

unsafe extern "C" {
    fn exception_vector_base();
}

/// Application dispatch contract called by the vector assembly
/// (`x0 = &mut TrapFrame, x1 = kind, x2 = source`).
#[unsafe(no_mangle)]
extern "C" fn trap_dispatch(tf: &mut TrapFrame, kind: u64, source: u64) {
    // IRQ: external IRQs and the GIC-delivered timer PPI.
    if kind == 1 {
        let mut handle = |irq: u32| crate::registries::IRQS.handle(irq);
        let mut timer_tick = || {
            kor_arch::increment_tick();
            kor_arch::handle_tick();
            if kor_sched::is_ready() {
                kor_sched::timer_tick();
                crate::sched::tick_current_process();
                kor_sched::preempt();
            }
        };
        kor_arch::dispatch_external(&mut handle, &mut timer_tick);
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

        // SVC from AArch64 (EC=0x15) out of lower EL — syscall.  (ELR
        // already points past the svc instruction.)
        if ec == 0x15 && source == 2 {
            let frame = tf as *mut TrapFrame as *mut u8 as *mut u8;
            crate::syscall::user_trap_enter(frame);
            let nr = tf.regs[8];
            let args = [
                tf.regs[0], tf.regs[1], tf.regs[2], tf.regs[3], tf.regs[4], tf.regs[5],
            ];
            let ret = crate::syscall::dispatch(nr, &args);
            tf.regs[0] = ret as usize;
            // Deliver any pending signal into the returning frame; a
            // default disposition terminates the task.
            if let Some(code) = crate::syscall::user_trap_status(frame) {
                crate::sched::exit_current(code);
            }
            crate::syscall::user_trap_leave(frame);
            return;
        }

        // Any other lower-EL (user) synchronous exception is a segfault.
        if source == 2 {
            crate::sched::exit_current(USER_SEGFAULT_STATUS);
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

/// Install the vector table: set `VBAR_EL1` and enable FPSIMD.
pub fn init() {
    const CPACR_FPEN_ALL: u64 = 0x300000;
    unsafe {
        // Enable FPSIMD/NEON access for EL0 and EL1 (FPEN=0b11).
        asm!("msr cpacr_el1, {0}", in(reg) CPACR_FPEN_ALL);
        asm!("isb");
        asm!("msr vbar_el1, {0}", in(reg) exception_vector_base as *const () as u64);
    }
    println!(
        "trap: VBAR_EL1 set to {:#x}",
        exception_vector_base as *const () as usize
    );
}
