//! `InterruptController` stub for x86_64.
//!
//! x86_64 routes device IRQs through the IDT (and PCI devices via MSI/legacy
//! INTx handled in PCI config space), not through a discoverable external-IRQ
//! controller reached from the trap handler.  Virtio-blk therefore runs in
//! polling mode on this architecture.  The stub lets the shared virtio path
//! compile uniformly and no-ops at runtime.

use kor::{InterruptController, TrapCallbacks};

/// x86_64 stub interrupt controller.
pub struct StubController;

/// Singleton instance installed by the binary crate.
pub static IC: StubController = StubController;

impl InterruptController for StubController {
    fn init(&self) {
        // LAPIC enable is part of IDT / trap setup; nothing controller-global.
    }

    fn enable_device_irq(&self, _interrupts: &[u32; 3]) -> Option<u32> {
        None
    }

    fn dispatch_external(&self, _callbacks: &dyn TrapCallbacks) {
        // x86_64 external IRQs go through IDT vectors, not a controller dispatch.
    }
}
