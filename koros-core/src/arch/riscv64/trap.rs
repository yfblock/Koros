//! riscv64 trap handling.
//!
//! Sets `stvec` to a hand-written assembly trampoline that saves all
//! registers, calls the Rust handler, then restores and `sret`.

use crate::println;
use core::arch::{asm, global_asm};

// TrapFrame layout must match what the assembly saves.
#[repr(C)]
struct TrapFrame {
    /// x0 – x31
    regs: [usize; 32],
    /// sstatus (csr 0x100)
    sstatus: usize,
    /// sepc   (csr 0x141)
    sepc: usize,
}

global_asm!(
    include_str!("trap.S"),
    tf_size = const core::mem::size_of::<TrapFrame>(),
);

unsafe extern "Rust" {
    fn kernelvec();
}

/// Called from the assembly trampoline with `a0 = &mut TrapFrame`.
#[unsafe(no_mangle)]
extern "C" fn handle_trap(tf: &mut TrapFrame) {
    let scause: usize;
    let stval: usize;
    unsafe {
        asm!("csrr {}, scause", out(reg) scause);
        asm!("csrr {}, stval",  out(reg) stval);
    }

    let is_interrupt = (scause >> 63) != 0;
    let code = scause & ((1 << 63) - 1);

    // Supervisor timer interrupt (cause 5): tick and return.
    if is_interrupt && code == 5 {
        crate::time::tick();
        return;
    }

    println!("");
    println!("=== TRAP ===");
    println!("scause  = {:#x} ({} #{})", scause, if is_interrupt { "Interrupt" } else { "Exception" }, code);
    println!("stval   = {:#x}", stval);
    println!("sepc    = {:#x}", tf.sepc);
    println!("sstatus = {:#x}", tf.sstatus);
    println!("");
    println!("Regs:  ra={:#x} sp={:#x} gp={:#x} tp={:#x}",
        tf.regs[1], tf.regs[2], tf.regs[3], tf.regs[4]);
    println!("       s0={:#x} s1={:#x} a0={:#x} a1={:#x}",
        tf.regs[8], tf.regs[9], tf.regs[10], tf.regs[11]);
    println!("       s2={:#x} s3={:#x} s4={:#x} s5={:#x}",
        tf.regs[18], tf.regs[19], tf.regs[20], tf.regs[21]);
    println!("       s6={:#x} s7={:#x} s8={:#x} s9={:#x}",
        tf.regs[22], tf.regs[23], tf.regs[24], tf.regs[25]);
    println!("      s10={:#x} s11={:#x} t0={:#x} t1={:#x}",
        tf.regs[26], tf.regs[27], tf.regs[5], tf.regs[6]);
    println!("       t2={:#x} t3={:#x} t4={:#x} t5={:#x}",
        tf.regs[7], tf.regs[28], tf.regs[29], tf.regs[30]);
    println!("       t6={:#x}", tf.regs[31]);

    panic!("Unhandled trap: scause={:#x}, sepc={:#x}", scause, tf.sepc);
}

/// Install the trap handler: set `stvec` to `kernelvec`.
pub fn init() {
    unsafe {
        // stvec = kernelvec (direct mode, not vectored)
        asm!("csrw stvec, {}", in(reg) kernelvec as *const () as usize);
    }
    println!("trap: stvec set to kernelvec ({:#x})", kernelvec as *const () as usize);
}
