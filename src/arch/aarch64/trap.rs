//! aarch64 trap handling.
//!
//! Sets `VBAR_EL1` to a hand-written vector table in assembly that
//! saves registers, calls the Rust handler, then restores and `eret`.

use crate::println;
use core::arch::{asm, global_asm};

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

global_asm!(include_str!("trap.S"));

unsafe extern "C" {
    fn exception_vector_base();
}

/// Called from assembly with `x0 = &mut TrapFrame, x1 = kind, x2 = source`.
#[unsafe(no_mangle)]
extern "C" fn handle_trap(tf: &mut TrapFrame, kind: u64, source: u64) {
    let esr: u64;
    let far: u64;
    unsafe {
        asm!("mrs {}, esr_el1", out(reg) esr);
        asm!("mrs {}, far_el1", out(reg) far);
    }

    let ec = esr >> 26;
    let iss = esr & 0x00FF_FFFF;

    let source_name = match source {
        0 => "CurrentEL SP_EL0",
        1 => "CurrentEL SP_ELx",
        2 => "LowerEL AArch64",
        3 => "LowerEL AArch32",
        _ => "Unknown",
    };

    let kind_name = match kind {
        0 => "Synchronous",
        1 => "IRQ",
        2 => "FIQ",
        3 => "SError",
        _ => "Unknown",
    };

    println!("");
    println!("=== TRAP ===");
    println!("Kind:   {} ({})", kind_name, kind);
    println!("Source: {} ({})", source_name, source);
    println!("ESR_EL1 = {:#x} (EC={:#06b}, ISS={:#x})", esr, ec, iss);
    println!("FAR_EL1 = {:#x}", far);
    println!("ELR_EL1 = {:#x}", tf.elr);
    println!("SPSR_EL1 = {:#x}", tf.spsr);
    println!("");

    // Print key registers
    println!("Register dump:");
    println!("  x0={:#018x}  x1={:#018x}  x2={:#018x}  x3={:#018x}", tf.regs[0], tf.regs[1], tf.regs[2], tf.regs[3]);
    println!("  x4={:#018x}  x5={:#018x}  x6={:#018x}  x7={:#018x}", tf.regs[4], tf.regs[5], tf.regs[6], tf.regs[7]);
    println!("  x8={:#018x}  x9={:#018x} x10={:#018x} x11={:#018x}", tf.regs[8], tf.regs[9], tf.regs[10], tf.regs[11]);
    println!(" x12={:#018x} x13={:#018x} x14={:#018x} x15={:#018x}", tf.regs[12], tf.regs[13], tf.regs[14], tf.regs[15]);
    println!(" x16={:#018x} x17={:#018x} x18={:#018x} x19={:#018x}", tf.regs[16], tf.regs[17], tf.regs[18], tf.regs[19]);
    println!(" x20={:#018x} x21={:#018x} x22={:#018x} x23={:#018x}", tf.regs[20], tf.regs[21], tf.regs[22], tf.regs[23]);
    println!(" x24={:#018x} x25={:#018x} x26={:#018x} x27={:#018x}", tf.regs[24], tf.regs[25], tf.regs[26], tf.regs[27]);
    println!(" x28={:#018x} x29={:#018x} x30={:#018x}", tf.regs[28], tf.regs[29], tf.regs[30]);
    println!(" sp={:#018x}  tpidr={:#018x}", tf.sp, tf.tpidr);

    panic!("Unhandled trap");
}

/// Install the trap handler: set `VBAR_EL1` to `exception_vector_base`.
pub fn init() {
    unsafe {
        asm!("msr vbar_el1, {}", in(reg) exception_vector_base as *const () as u64);
    }
    println!("trap: VBAR_EL1 set to {:#x}", exception_vector_base as *const () as usize);
}
