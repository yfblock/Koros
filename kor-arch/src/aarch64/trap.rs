//! aarch64 trap handling.
//!
//! Sets `VBAR_EL1` to a hand-written vector table in assembly that
//! saves registers, calls the Rust handler, then restores and `eret`.

use kor::println;
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
    // IRQ (kind 1): dispatch through the interrupt controller.
    if kind == 1 {
        kor::dispatch_external();
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
                unsafe extern "Rust" {
                    fn korix_set_current_tf(tf: *mut u8);
                }
                unsafe { korix_set_current_tf(tf as *mut TrapFrame as *mut u8); }
                let nr = tf.regs[8];       // x8 = syscall number
                let args = [
                    tf.regs[0],            // x0
                    tf.regs[1],            // x1
                    tf.regs[2],            // x2
                    tf.regs[3],            // x3
                    tf.regs[4],            // x4
                    tf.regs[5],            // x5
                ];
                let ret = handler(nr, &args);
                tf.regs[0] = ret as usize;  // x0 = return value
                tf.elr += 4;                // skip SVC instruction
                return;
            }
        }

        println!("");
        println!("=== TRAP ===");
        println!("ESR_EL1 = {:#x} (EC={:#x}, ISS={:#x})", esr, ec, esr & 0xFFFFFF);
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
    println!("trap: VBAR_EL1 set to {:#x}", exception_vector_base as *const () as usize);
}
