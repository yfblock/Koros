//! Local interrupt control.
//!
//! A thin, architecture-neutral interface over each CPU's global interrupt
//! flag (riscv `sstatus.SIE`, x86 `RFLAGS.IF`, aarch64 `DAIF.I`, loongarch
//! `CRMD.IE`).  Device-level interrupt sources are armed elsewhere (e.g.
//! [`crate::time::init`]); this module only gates whether the CPU takes them.

#[cfg(target_arch = "riscv64")]
use crate::arch::riscv64::irq as arch_irq;
#[cfg(target_arch = "x86_64")]
use crate::arch::x86_64::irq as arch_irq;
#[cfg(target_arch = "aarch64")]
use crate::arch::aarch64::irq as arch_irq;
#[cfg(target_arch = "loongarch64")]
use crate::arch::loongarch64::irq as arch_irq;

/// Enable interrupts on the current CPU.
#[inline]
pub fn enable() {
    arch_irq::enable();
}

/// Disable interrupts on the current CPU.
#[inline]
pub fn disable() {
    arch_irq::disable();
}

/// Whether interrupts are currently enabled on this CPU.
#[inline]
pub fn is_enabled() -> bool {
    arch_irq::is_enabled()
}

/// Run `f` with interrupts disabled, restoring the previous state afterwards.
#[inline]
pub fn without<R>(f: impl FnOnce() -> R) -> R {
    let was_enabled = is_enabled();
    disable();
    let result = f();
    if was_enabled {
        enable();
    }
    result
}
