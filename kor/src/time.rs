//! Timer tick counter.  The arch timer is armed via the `ArchProvider`
//! (`timer_init`/`handle_tick`); the composition layer's `TrapCallbacks`
//! increments this counter and drives the scheduler.

use core::sync::atomic::{AtomicU64, Ordering};

/// Timer ticks since boot (incremented by the trap-callback `on_timer`).
static TICKS: AtomicU64 = AtomicU64::new(0);

/// Timer interrupts per second (the tick rate the arch code programs).
pub const TICK_HZ: u64 = 100;

/// Increment the tick counter (called from `TrapCallbacks::on_timer`).
#[inline]
pub fn increment_tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

/// Ticks elapsed since boot.
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}
