//! loongarch64 trap handling.
//!
//! Sets `CSR.EENTRY` to a hand-written trampoline that saves all
//! registers, calls the Rust handler, then restores and `ertn`.

use crate::println;
use core::arch::{asm, global_asm};

#[repr(C)]
struct TrapFrame {
    /// $r0 – $r31 (32 general registers)
    regs: [usize; 32],
    /// PRMD (CSR 0x1)
    prmd: usize,
    /// ERA  (CSR 0x6)
    era: usize,
}

global_asm!(
    include_str!("trap.S"),
    tf_size = const core::mem::size_of::<TrapFrame>(),
);

unsafe extern "C" {
    fn trap_vector_base();
}

/// Called from the assembly trampoline with `$a0 = &mut TrapFrame`.
#[unsafe(no_mangle)]
extern "C" fn handle_trap(tf: &mut TrapFrame) {
    let estat: usize;
    let badv: usize;
    unsafe {
        asm!("csrrd {}, 0x5", out(reg) estat);  // ESTAT
        asm!("csrrd {}, 0x7", out(reg) badv);   // BADV
    }

    let is_interrupt = (estat >> 2) & 0x3f != 0;  // bit 12:0 for hw interrupt pending
    let exc_code = estat & 0x3f;  // bits 5:0 = ECODE

    println!("");
    println!("=== TRAP ===");
    println!("ESTAT    = {:#x} (ECODE={}, {} )", estat, exc_code,
        if is_interrupt { "Interrupt" } else { "Exception" });
    println!("BADV     = {:#x}", badv);
    println!("ERA      = {:#x}", tf.era);
    println!("PRMD     = {:#x}", tf.prmd);
    println!("");

    // Print key registers
    println!("Register dump:");
    println!("  ra={:#018x} tp={:#018x} sp={:#018x} fp={:#018x}",
        tf.regs[1], tf.regs[2], tf.regs[3], tf.regs[22]);
    println!("  a0={:#018x} a1={:#018x} a2={:#018x} a3={:#018x}",
        tf.regs[4], tf.regs[5], tf.regs[6], tf.regs[7]);
    println!("  a4={:#018x} a5={:#018x} a6={:#018x} a7={:#018x}",
        tf.regs[8], tf.regs[9], tf.regs[10], tf.regs[11]);
    println!("  t0={:#018x} t1={:#018x} t2={:#018x} t3={:#018x}",
        tf.regs[12], tf.regs[13], tf.regs[14], tf.regs[15]);
    println!("  t4={:#018x} t5={:#018x} t6={:#018x} t7={:#018x}",
        tf.regs[16], tf.regs[17], tf.regs[18], tf.regs[19]);
    println!("  t8={:#018x} r21={:#018x}s0={:#018x} s1={:#018x}",
        tf.regs[20], tf.regs[21], tf.regs[23], tf.regs[24]);
    println!("  s2={:#018x} s3={:#018x} s4={:#018x} s5={:#018x}",
        tf.regs[25], tf.regs[26], tf.regs[27], tf.regs[28]);
    println!("  s6={:#018x} s7={:#018x} s8={:#018x}",
        tf.regs[29], tf.regs[30], tf.regs[31]);

    panic!("Unhandled trap");
}

/// Install the trap handler: set `CSR.EENTRY` to `trap_vector_base`.
pub fn init() {
    let mut eentry = trap_vector_base as *const () as usize;
    unsafe {
        asm!("csrwr {}, 0xc", inout(reg) eentry);  // EENTRY = 0xc
    }
    let _ = eentry; // old CSR value unused
    println!("trap: EENTRY set to {:#x}", trap_vector_base as *const () as usize);
}
