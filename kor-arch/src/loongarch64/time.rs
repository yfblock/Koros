//! loongarch64 timer: the constant (stable) timer, periodic mode.

use core::arch::asm;

// CSR numbers.
const CSR_ECFG: usize = 0x4; // interrupt enable: bit 11 = timer (TI)
const CSR_TCFG: usize = 0x41; // timer config
const CSR_TICLR: usize = 0x44; // timer interrupt clear

const TCFG_EN: usize = 1 << 0;
const TCFG_PERIODIC: usize = 1 << 1;
const ECFG_TI: usize = 1 << 11;

/// Constant-timer frequency from CPUCFG words 4 (base) and 5 (mul/div).
fn timer_freq() -> u64 {
    let base: usize;
    let muldiv: usize;
    // SAFETY: CPUCFG is a read-only configuration instruction.
    unsafe {
        asm!("cpucfg {}, {}", out(reg) base, in(reg) 4usize);
        asm!("cpucfg {}, {}", out(reg) muldiv, in(reg) 5usize);
    }
    let cc_freq = (base & 0xffff_ffff) as u64;
    let mul = (muldiv & 0xffff) as u64;
    let div = ((muldiv >> 16) & 0xffff) as u64;
    if cc_freq == 0 || mul == 0 || div == 0 {
        100_000_000 // fallback
    } else {
        cc_freq * mul / div
    }
}

fn csr_write(csr: usize, val: usize) {
    // `csrwr` swaps rd with the CSR; we discard the old value.
    match csr {
        CSR_ECFG => unsafe { asm!("csrwr {}, 0x4", inout(reg) val => _) },
        CSR_TCFG => unsafe { asm!("csrwr {}, 0x41", inout(reg) val => _) },
        CSR_TICLR => unsafe { asm!("csrwr {}, 0x44", inout(reg) val => _) },
        _ => unreachable!(),
    }
}

fn csr_read(csr: usize) -> usize {
    let v: usize;
    match csr {
        CSR_ECFG => unsafe { asm!("csrrd {}, 0x4", out(reg) v) },
        _ => unreachable!(),
    }
    v
}

/// Program the periodic timer and enable the timer interrupt *source*.
/// Global interrupts are enabled separately via [`kor::irq::enable`].
pub fn init() {
    let interval = (timer_freq() / kor::time::TICK_HZ) as usize & !0x3;
    csr_write(CSR_TCFG, interval | TCFG_EN | TCFG_PERIODIC);
    csr_write(CSR_ECFG, csr_read(CSR_ECFG) | ECFG_TI);
}

/// Clear the timer interrupt (the periodic timer auto-reloads).
pub fn handle_tick() {
    csr_write(CSR_TICLR, 1);
}
