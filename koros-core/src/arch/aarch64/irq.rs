//! aarch64 local interrupt control via `DAIF.I` (IRQ mask, bit 7).

use core::arch::asm;

const DAIF_I: u64 = 1 << 7;

pub fn enable() {
    // SAFETY: clears the IRQ mask (DAIF.I).
    unsafe { asm!("msr daifclr, #2", options(nomem, nostack)) };
}

pub fn disable() {
    // SAFETY: sets the IRQ mask (DAIF.I).
    unsafe { asm!("msr daifset, #2", options(nomem, nostack)) };
}

pub fn is_enabled() -> bool {
    let daif: u64;
    // SAFETY: reads the DAIF state.
    unsafe { asm!("mrs {}, daif", out(reg) daif) };
    // A set I bit means IRQs are masked (disabled).
    daif & DAIF_I == 0
}
