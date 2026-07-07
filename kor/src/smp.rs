//! Multi-core support: online tracking + secondary bring-up orchestration.
//!
//! CPU identity / `wait_for_interrupt` / `start_secondaries` live in the
//! `ArchProvider`; the secondary CPU entry point (`secondary_entry`) is owned
//! by the composition layer (`koros`).  This module tracks how many CPUs have
//! come online and waits for secondaries to register.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Maximum number of CPUs the kernel tracks.
pub const MAX_CPUS: usize = 8;

/// CPUs currently online, including the boot CPU (which starts counted).
static ONLINE: AtomicUsize = AtomicUsize::new(1);

/// Number of CPUs that have come online (including the boot CPU).
pub fn online_count() -> usize {
    ONLINE.load(Ordering::Acquire)
}

/// Record a secondary CPU as online (called from `secondary_entry` in `koros`).
pub fn register_online() {
    ONLINE.fetch_add(1, Ordering::AcqRel);
}

/// Bring up the secondary CPUs and wait (bounded) for them to register.
/// Returns the total number of CPUs online (including the boot CPU).
pub fn boot_secondaries() -> usize {
    let started = crate::arch::current().start_secondaries();
    let target = 1 + started;
    let mut spins: u64 = 0;
    while online_count() < target && spins < 2_000_000_000 {
        core::hint::spin_loop();
        spins += 1;
    }
    online_count()
}
