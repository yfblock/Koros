//! `InterruptController` trait + installed-controller registry.

use spin::Once;

use crate::trap_callbacks::TrapCallbacks;

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
    /// Acknowledge and dispatch all pending external interrupts.
    fn dispatch_external(&self, callbacks: &dyn TrapCallbacks);
}

static INT_CONTROLLER: Once<&'static dyn InterruptController> = Once::new();

/// Install the interrupt controller (may be omitted on stub platforms).
pub fn install_controller(ic: &'static dyn InterruptController) {
    INT_CONTROLLER.call_once(|| ic);
}

/// The installed interrupt controller, if any.
pub fn controller() -> Option<&'static dyn InterruptController> {
    INT_CONTROLLER.get().copied()
}

/// Convenience: enable a device IRQ through the installed controller.
pub fn enable_device_irq(interrupts: &[u32; 3]) -> Option<u32> {
    controller().and_then(|ic| ic.enable_device_irq(interrupts))
}
