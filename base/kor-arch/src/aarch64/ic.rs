//! `InterruptController` implementation for aarch64 — the QEMU `virt` GICv2.
//!
//! [`GicController::init`] brings up the distributor + CPU interface (called
//! per-CPU).  [`dispatch_external`] reads the IAR, routes the generic-timer
//! PPI to the timer-tick callback and everything else to
//! the IRQ callback, then signals EOI.


use crate::InterruptController;

/// The aarch64 GICv2 interrupt controller — a zero-sized marker.
pub struct GicController;

/// Singleton instance installed by the binary crate.
pub static GIC: GicController = GicController;

impl InterruptController for GicController {
    fn init(&self) {
        super::gic::init();
    }

    fn enable_device_irq(&self, interrupts: &[u32; 3]) -> Option<u32> {
        // GIC `interrupts` = <type, number, flags>; SPI (type 0) -> INTID num+32.
        if interrupts[0] == 0 {
            let intid = interrupts[1] + 32;
            super::gic::enable_spi(intid);
            kor::println!("virtio-blk: IRQ {} via GIC", intid);
            Some(intid)
        } else {
            None
        }
    }

    fn dispatch_external(&self, handle_irq: &mut dyn FnMut(u32), timer_tick: &mut dyn FnMut()) {
        let iar = super::gic::read_iar();
        let intid = iar & 0x3ff;
        // 1020+ are spurious; ignore without EOI.
        if intid >= 1020 {
            return;
        }
        if intid == super::time::TIMER_INTID {
            // Timer: EOI first, then run the tick/preempt path.
            super::gic::write_eoir(iar);
            timer_tick();
        } else {
            handle_irq(intid);
            super::gic::write_eoir(iar);
        }
    }
}
