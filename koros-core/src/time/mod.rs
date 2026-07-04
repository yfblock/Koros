//! Timer / tick handling.
//!
//! Each architecture provides a uniform `time` interface under
//! `arch/<arch>/time.rs`:
//!
//! - `init()` — program the first timer deadline and enable the timer
//!   interrupt (and global interrupts) on the current CPU.
//! - `handle_tick()` — acknowledge the timer interrupt and program the next
//!   deadline; called from the arch trap handler when a timer interrupt fires.
//!
//! The per-arch trap handler recognises its timer interrupt and calls
//! [`tick`].  The handler must stay minimal (no locks, no console output) so
//! it is safe to fire while other code holds the console lock.

use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_arch = "riscv64")]
use crate::arch::riscv64::time as arch_time;
#[cfg(target_arch = "x86_64")]
use crate::arch::x86_64::time as arch_time;
#[cfg(target_arch = "aarch64")]
use crate::arch::aarch64::time as arch_time;
#[cfg(target_arch = "loongarch64")]
use crate::arch::loongarch64::time as arch_time;

/// Timer ticks since boot (incremented on every timer interrupt).
static TICKS: AtomicU64 = AtomicU64::new(0);

/// Timer interrupts per second (the tick rate the arch code programs).
pub const TICK_HZ: u64 = 100;

/// Program the timer and enable interrupts on the current CPU.
pub fn init() {
    arch_time::init();
}

/// Ticks elapsed since boot.
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Handle one timer interrupt: count it and program the next deadline.
/// Called from the arch trap handler.
pub fn tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
    arch_time::handle_tick();
    crate::sched::timer_tick();
}
