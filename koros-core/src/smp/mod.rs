//! Multi-core (SMP) support: CPU identity, secondary bring-up, online tracking.
//!
//! Architecture-specific bring-up lives in `arch/<arch>/smp.rs` behind a
//! uniform interface (`cpu_id`, `start_secondaries`, `wait_for_interrupt`).
//! There is no scheduler yet, so a secondary CPU just installs its trap
//! vector, registers itself online, and idles.

use core::sync::atomic::{AtomicUsize, Ordering};

#[cfg(target_arch = "riscv64")]
use crate::arch::riscv64::smp as arch_smp;
#[cfg(target_arch = "x86_64")]
use crate::arch::x86_64::smp as arch_smp;
#[cfg(target_arch = "aarch64")]
use crate::arch::aarch64::smp as arch_smp;
#[cfg(target_arch = "loongarch64")]
use crate::arch::loongarch64::smp as arch_smp;

/// Maximum number of CPUs the kernel tracks.
pub const MAX_CPUS: usize = 8;

/// CPUs currently online, including the boot CPU (which starts counted).
static ONLINE: AtomicUsize = AtomicUsize::new(1);

/// Hardware identifier of the current CPU (hart id / APIC id / core id).
pub fn cpu_id() -> usize {
    arch_smp::cpu_id()
}

/// Number of CPUs that have come online (including the boot CPU).
pub fn online_count() -> usize {
    ONLINE.load(Ordering::Acquire)
}

/// Idle the current CPU until an interrupt arrives.
pub fn wait_for_interrupt() {
    arch_smp::wait_for_interrupt();
}

/// Bring up the secondary CPUs and wait (bounded) for them to register.
/// Returns the total number of CPUs online (including the boot CPU).
///
/// The bound is only a safety net against a CPU that never comes up; with
/// per-vCPU host threads (`-accel tcg,thread=multi`) the secondaries register
/// quickly and we exit as soon as the expected count is reached.
pub fn boot_secondaries() -> usize {
    let started = arch_smp::start_secondaries();
    let target = 1 + started;
    let mut spins: u64 = 0;
    while online_count() < target && spins < 2_000_000_000 {
        core::hint::spin_loop();
        spins += 1;
    }
    online_count()
}

/// Entry point for a secondary CPU, called from the arch secondary boot code
/// once paging and the CPU's stack are set up.  Never returns.
pub extern "C" fn secondary_entry(id: usize) -> ! {
    crate::trap::init();
    ONLINE.fetch_add(1, Ordering::AcqRel);
    crate::println!("cpu {} online", id);

    // Arm this CPU's timer and enable interrupts, then idle until the boot CPU
    // has initialised the scheduler.  Interrupts must be on so `wait_for_interrupt`
    // (`hlt`/`wfi`) is woken by the timer instead of blocking forever; the timer
    // handler no-ops until the scheduler is ready.
    crate::time::init();
    crate::irq::enable();
    while !crate::sched::is_ready() {
        arch_smp::wait_for_interrupt();
    }
    crate::sched::init_this_cpu();
    crate::sched::idle_loop();
}
