//! `InterruptController` trait.  The active controller comes from the
//! crate's cfg_select `interrupt_controller()` selector — no registry.

/// Routes device interrupts (claim/enable/dispatch) for the current platform.
///
/// On architectures without a discoverable external-IRQ controller a stub
/// returns `None` from [`enable_device_irq`] and no-ops [`dispatch_external`],
/// so virtio falls back to polling.
pub trait InterruptController: Send + Sync {
    /// One-time controller initialisation (e.g. GIC distributor enable).
    fn init(&self);
    /// Enable a device IRQ described by FDT `interrupts` cells.
    fn enable_device_irq(&self, interrupts: &[u32; 3]) -> Option<u32>;
    /// Acknowledge and dispatch all pending external interrupts: each
    /// claimed device IRQ goes to `handle_irq`; on controllers where the
    /// generic timer arrives as a PPI (GIC), a timer interrupt calls
    /// `timer_tick` instead.
    fn dispatch_external(&self, handle_irq: &mut dyn FnMut(u32), timer_tick: &mut dyn FnMut());
}

/// Convenience: enable a device IRQ (described by FDT `interrupts` cells)
/// through the active controller.
pub fn enable_device_irq(interrupts: &[u32; 3]) -> Option<u32> {
    crate::interrupt_controller().enable_device_irq(interrupts)
}
