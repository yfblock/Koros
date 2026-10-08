//! riscv64 local interrupt control via `sstatus.SIE`.

use core::arch::asm;

const SSTATUS_SIE: usize = 1 << 1;

pub fn enable() {
    // SAFETY: sets the supervisor interrupt-enable bit.
    unsafe { asm!("csrs sstatus, {}", in(reg) SSTATUS_SIE) };
}

pub fn disable() {
    // SAFETY: clears the supervisor interrupt-enable bit.
    unsafe { asm!("csrc sstatus, {}", in(reg) SSTATUS_SIE) };
}

pub fn is_enabled() -> bool {
    let sstatus: usize;
    // SAFETY: reads a status CSR.
    unsafe { asm!("csrr {}, sstatus", out(reg) sstatus) };
    sstatus & SSTATUS_SIE != 0
}
