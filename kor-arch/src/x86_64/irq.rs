//! x86_64 local interrupt control via `RFLAGS.IF`.

use core::arch::asm;

const RFLAGS_IF: u64 = 1 << 9;

pub fn enable() {
    // SAFETY: sets the interrupt flag.
    unsafe { asm!("sti", options(nomem, nostack)) };
}

pub fn disable() {
    // SAFETY: clears the interrupt flag.
    unsafe { asm!("cli", options(nomem, nostack)) };
}

pub fn is_enabled() -> bool {
    let flags: u64;
    // SAFETY: reads RFLAGS.
    unsafe { asm!("pushfq; pop {}", out(reg) flags, options(nomem)) };
    flags & RFLAGS_IF != 0
}
