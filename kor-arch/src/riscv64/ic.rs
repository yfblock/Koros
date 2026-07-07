//! `InterruptController` implementation for riscv64 — the QEMU `virt` PLIC.
//!
//! The PLIC has no global one-shot init (priority/enables are per-source,
//! SEIE is per-hart), so [`PlicController::init`] is a no-op; device IRQs are
//! armed one-by-one via [`enable_device_irq`] and dispatched by
//! [`dispatch_external`], which claims/ completes each pending source.

use kor::{InterruptController, TrapCallbacks};

/// The riscv64 PLIC interrupt controller — a zero-sized marker.
pub struct PlicController;

/// Singleton instance installed by the binary crate.
pub static PLIC: PlicController = PlicController;

impl InterruptController for PlicController {
    fn init(&self) {
        // No global PLIC init; SEIE is enabled per-hart in `enable_device_irq`.
    }

    fn enable_device_irq(&self, interrupts: &[u32; 3]) -> Option<u32> {
        // PLIC uses a single `interrupts` cell: the source number.
        let irq = interrupts[0];
        if irq == 0 {
            return None;
        }
        let hart = super::smp::cpu_id();
        super::plic::enable(irq, hart);
        super::plic::enable_seie();
        kor::println!("virtio-blk: IRQ {} via PLIC on hart {}", irq, hart);
        Some(irq)
    }

    fn dispatch_external(&self, callbacks: &dyn TrapCallbacks) {
        let ctx = super::plic::context(super::smp::cpu_id());
        let claim = super::plic::claim_reg(ctx);
        loop {
            // SAFETY: claim register read returns the highest-priority pending IRQ.
            let irq = unsafe { claim.read_volatile() };
            if irq == 0 {
                break;
            }
            callbacks.on_external(irq);
            // SAFETY: writing the IRQ back signals completion.
            unsafe { claim.write_volatile(irq) };
        }
    }
}
