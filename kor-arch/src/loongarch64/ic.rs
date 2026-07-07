//! `InterruptController` stub for loongarch64.
//!
//! The loongarch64 extioi / PCH-PIC routing is not implemented yet, so
//! virtio-blk runs in polling mode.  The stub lets the shared virtio path
//! compile uniformly and no-ops at runtime.

use kor::{InterruptController, TrapCallbacks};

/// loongarch64 stub interrupt controller.
pub struct StubController;

/// Singleton instance installed by the binary crate.
pub static IC: StubController = StubController;

impl InterruptController for StubController {
    fn init(&self) {}

    fn enable_device_irq(&self, _interrupts: &[u32; 3]) -> Option<u32> {
        None
    }

    fn dispatch_external(&self, _callbacks: &dyn TrapCallbacks) {}
}
