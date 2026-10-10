//! riscv64 trap handling for korix: vector composed from the kor-arch
//! fragment, dispatch implemented here — timer tick, external IRQs, the
//! syscall entry (with PC-advance policy), and signal delivery on the way
//! out of a user-origin trap.

use core::arch::{asm, global_asm};

use kor::println;
use kor_arch::TrapFrame;

global_asm!(
    kor_arch::trap_user_vec_asm!(),
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

    // Supervisor timer interrupt: tick, re-arm, drain console input, preempt.
    if is_interrupt && code == 5 {
        kor_arch::increment_tick();
        kor_arch::handle_tick();
        if kor_sched::is_ready() {
            kor_sched::timer_tick();
            kor_sched::preempt();
        }
        crate::syscall::rx_fill();
        return;
    }
    // Supervisor external interrupt: claim/handle/complete via the PLIC.
    if is_interrupt && code == 9 {
        let mut handle = |irq: u32| crate::registries::IRQS.handle(irq);
        let mut timer_tick = || {};
        kor_arch::dispatch_external(&mut handle, &mut timer_tick);
        return;
    }

    // Environment call from U-mode: run the syscall.
    if !is_interrupt && code == 8 && tf.sstatus & (1 << 8) == 0 {
        let frame = tf as *mut TrapFrame as *mut u8;
        crate::syscall::user_trap_enter(frame);
        let nr = tf.regs[17];
        let args = [
            tf.regs[10],
            tf.regs[11],
            tf.regs[12],
            tf.regs[13],
            tf.regs[14],
            tf.regs[15],
        ];
        let ret = crate::syscall::dispatch(nr, &args);
        tf.regs[10] = ret as usize;
        // PC-advance policy (asm-generic numbers, native to riscv64):
        // rt_sigreturn restores its own saved PC, and a successful execve
        // installs a brand-new context — neither may bump sepc.
        const NR_RT_SIGRETURN: usize = 139;
        const NR_EXECVE: usize = 221;
        if nr != NR_RT_SIGRETURN && (nr != NR_EXECVE || ret < 0) {
            tf.sepc += 4;
        }
        // Deliver any pending signal into the returning frame; a default
        // disposition terminates the task instead of returning.
        if let Some(code) = crate::syscall::user_trap_status(frame) {
            crate::sched::exit_current(code);
        }
        crate::syscall::user_trap_leave(frame);
        return;
    }

    // Any other exception raised from U-mode is a user fault (bad memory
    // access, illegal instruction, …): terminate the process like a
    // SIGSEGV instead of taking the kernel down with it.
    if !is_interrupt && tf.sstatus & (1 << 8) == 0 {
        const USER_SEGFAULT_STATUS: i32 = 128 + 11; // WIFSIGNLED-style SIGSEGV
        println!(
            "[korix] user fault: scause={:#x} stval={:#x} sepc={:#x} last-syscall={} — exiting",
            scause,
            stval,
            tf.sepc,
            crate::syscall::LAST_SYSCALL.load(core::sync::atomic::Ordering::Relaxed)
        );
        crate::sched::exit_current(USER_SEGFAULT_STATUS);
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
