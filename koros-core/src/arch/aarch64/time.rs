//! aarch64 timer: the EL1 physical generic timer (CNTP), delivered as a PPI
//! through the GICv2 (see [`super::gic`]).

use core::sync::atomic::{AtomicU64, Ordering};

/// EL1 non-secure physical timer PPI (PPI 14 -> INTID 30).
pub(super) const TIMER_INTID: u32 = 30;

/// Countdown value programmed each period (set in `init`).
static INTERVAL: AtomicU64 = AtomicU64::new(0);

fn cntfrq() -> u64 {
    let f: u64;
    // SAFETY: reads the read-only counter-frequency register.
    unsafe { core::arch::asm!("mrs {}, cntfrq_el0", out(reg) f) };
    f
}

fn set_timer(interval: u64) {
    // SAFETY: programs the EL1 physical timer downcounter and enables it.
    unsafe {
        core::arch::asm!("msr cntp_tval_el0, {}", in(reg) interval);
        core::arch::asm!("msr cntp_ctl_el0, {}", in(reg) 1u64); // ENABLE, unmasked
    }
}

/// Initialise the GIC (per CPU), enable the timer PPI, and start the timer.
pub fn init() {
    super::gic::init();
    super::gic::enable_ppi(TIMER_INTID);

    let interval = cntfrq() / crate::time::TICK_HZ;
    INTERVAL.store(interval, Ordering::Relaxed);
    set_timer(interval);
    // Global interrupts are enabled separately via `crate::irq::enable`.
}

/// Reprogram the timer for the next period (called from the tick path; the
/// GIC ack/EOI is handled by [`super::gic::dispatch`]).
pub fn handle_tick() {
    set_timer(INTERVAL.load(Ordering::Relaxed));
}
