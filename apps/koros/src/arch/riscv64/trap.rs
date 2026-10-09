//! riscv64 trap handling for koros: vector composed from the kor-arch
//! fragment, dispatch implemented here (timer tick + external IRQs; koros
//! has no user mode, so every syscall/user exception is fatal).

use core::arch::{asm, global_asm};

use kor::println;
use kor_arch::TrapFrame;

global_asm!(
    kor_arch::trap_kernel_vec_asm!(),
    tf_size = const core::mem::size_of::<TrapFrame>(),
);

unsafe extern "Rust" {
    fn kernelvec();
}

/// Application dispatch contract called by the vector assembly.
#[unsafe(no_mangle)]
extern "C" fn trap_dispatch(tf: &mut TrapFrame) {
    let scause: usize;
    let stval: usize;
    unsafe {
        asm!("csrr {}, scause", out(reg) scause);
        asm!("csrr {}, stval", out(reg) stval);
    }
    let is_interrupt = (scause >> 63) != 0;
    let code = scause & ((1 << 63) - 1);

    // Supervisor timer interrupt: tick, re-arm, and preempt.
    if is_interrupt && code == 5 {
        kor_arch::increment_tick();
        kor_arch::handle_tick();
        if kor_sched::is_ready() {
            kor_sched::timer_tick();
            kor_sched::preempt();
        }
        return;
    }
    // Supervisor external interrupt: claim/handle/complete via the PLIC.
    if is_interrupt && code == 9 {
        let mut handle = |irq: u32| crate::registries::IRQS.handle(irq);
        let mut timer_tick = || {};
        kor_arch::dispatch_external(&mut handle, &mut timer_tick);
        return;
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

/// Point `stvec` at the composed vector.
pub fn init() {
    // SAFETY: writing the vector base register; the vector is mapped.
    unsafe { asm!("csrw stvec, {}", in(reg) kernelvec as *const () as usize) };
    println!(
        "trap: stvec set to kernelvec ({:#x})",
        kernelvec as *const () as usize
    );
}
