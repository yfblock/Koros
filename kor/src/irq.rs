//! Local interrupt control — thin wrappers over the installed `ArchProvider`.

/// Enable interrupts on the current CPU.
#[inline]
pub fn enable() {
    crate::arch::current().irq_enable();
}

/// Disable interrupts on the current CPU.
#[inline]
pub fn disable() {
    crate::arch::current().irq_disable();
}

/// Whether interrupts are currently enabled on this CPU.
#[inline]
pub fn is_enabled() -> bool {
    crate::arch::current().irq_is_enabled()
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
