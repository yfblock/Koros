//! riscv64 timer: supervisor timer interrupt driven by the SBI TIME extension.

use core::arch::asm;

/// QEMU `virt` timebase frequency (10 MHz).
const TIMER_FREQ: u64 = 10_000_000;
/// Timer period in ticks of the `time` CSR.
const INTERVAL: u64 = TIMER_FREQ / crate::time::TICK_HZ;

// SBI TIME extension.
const SBI_EXT_TIME: usize = 0x5449_4D45;
const SBI_FN_SET_TIMER: usize = 0;

// `sie` bit for the supervisor timer interrupt source.
const SIE_STIE: usize = 1 << 5;

fn read_time() -> u64 {
    let t: u64;
    // SAFETY: `rdtime` reads the read-only time CSR.
    unsafe { asm!("rdtime {}", out(reg) t) };
    t
}

/// SBI `set_timer(deadline)` — schedules the next supervisor timer interrupt.
fn sbi_set_timer(deadline: u64) {
    // SAFETY: SBI ecall per the TIME extension calling convention.
    unsafe {
        asm!(
            "ecall",
            in("a7") SBI_EXT_TIME,
            in("a6") SBI_FN_SET_TIMER,
            inlateout("a0") deadline as usize => _,
            out("a1") _,
        );
    }
}

/// Program the first deadline and enable the timer interrupt *source*.
/// Global interrupts are enabled separately via [`crate::irq::enable`].
pub fn init() {
    sbi_set_timer(read_time() + INTERVAL);
    // SAFETY: enable the supervisor timer interrupt source.
    unsafe { asm!("csrs sie, {}", in(reg) SIE_STIE) };
}

/// Acknowledge the timer by programming the next deadline.
pub fn handle_tick() {
    sbi_set_timer(read_time() + INTERVAL);
}
