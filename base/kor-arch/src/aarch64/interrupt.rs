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
        init();
    }

    fn enable_device_irq(&self, interrupts: &[u32; 3]) -> Option<u32> {
        // GIC `interrupts` = <type, number, flags>; SPI (type 0) -> INTID num+32.
        if interrupts[0] == 0 {
            let intid = interrupts[1] + 32;
            enable_spi(intid);
            kor::println!("virtio-blk: IRQ {} via GIC", intid);
            Some(intid)
        } else {
            None
        }
    }

    fn dispatch_external(&self, handle_irq: &mut dyn FnMut(u32), timer_tick: &mut dyn FnMut()) {
        let iar = read_iar();
        let intid = iar & 0x3ff;
        // 1020+ are spurious; ignore without EOI.
        if intid >= 1020 {
            return;
        }
        if intid == super::time::TIMER_INTID {
            // Timer: EOI first, then run the tick/preempt path.
            write_eoir(iar);
            timer_tick();
        } else {
            handle_irq(intid);
            write_eoir(iar);
        }
    }
}

// --- Local IRQ mask (DAIF.I / sie) ---

use core::arch::asm;

const DAIF_I: u64 = 1 << 7;

pub fn enable() {
    // SAFETY: clears the IRQ mask (DAIF.I).
    unsafe { asm!("msr daifclr, #2", options(nomem, nostack)) };
}

pub fn disable() {
    // SAFETY: sets the IRQ mask (DAIF.I).
    unsafe { asm!("msr daifset, #2", options(nomem, nostack)) };
}

pub fn is_enabled() -> bool {
    let daif: u64;
    // SAFETY: reads the DAIF state.
    unsafe { asm!("mrs {}, daif", out(reg) daif) };
    // A set I bit means IRQs are masked (disabled).
    daif & DAIF_I == 0
}


// --- GIC registers ---

const GICD_BASE: usize = 0x0800_0000;
const GICC_BASE: usize = 0x0801_0000;

const GICD_CTLR: usize = 0x000;
const GICD_ISENABLER: usize = 0x100;
const GICD_IPRIORITYR: usize = 0x400;
const GICD_ITARGETSR: usize = 0x800;

const GICC_CTLR: usize = 0x000;
const GICC_PMR: usize = 0x004;
const GICC_IAR: usize = 0x00C;
const GICC_EOIR: usize = 0x010;

fn gicd(off: usize) -> kor::KernelAddr {
    kor::PhysAddr::new(GICD_BASE + off).to_ka()
}

fn gicc(off: usize) -> kor::KernelAddr {
    kor::PhysAddr::new(GICC_BASE + off).to_ka()
}

fn gicc_read(off: usize) -> u32 {
    // SAFETY: GIC CPU-interface MMIO is mapped in the direct map.
    gicc(off).read_u32()
}

fn gicc_write(off: usize, val: u32) {
    // SAFETY: GIC CPU-interface MMIO is mapped in the direct map.
    gicc(off).write_u32(val);
}

/// Read the interrupt acknowledge register (returns the active INTID).
pub fn read_iar() -> u32 {
    gicc_read(GICC_IAR)
}

/// Signal end-of-interrupt for the acknowledged `iar` value.
pub fn write_eoir(iar: u32) {
    gicc_write(GICC_EOIR, iar)
}

/// Enable an interrupt id (sets priority to 0 = highest, sets the enable bit).
fn enable_intid(intid: u32) {
    kor::PhysAddr::new(GICD_BASE + GICD_IPRIORITYR + intid as usize).to_ka().write_u8(0);
    let en = gicd(GICD_ISENABLER + (intid as usize / 32) * 4);
    en.write_u32(en.read_u32() | (1 << (intid % 32)));
}

/// Distributor + CPU-interface enable and priority mask.  Called per CPU.
pub fn init() {
    // SAFETY: GIC MMIO is mapped.
    gicd(GICD_CTLR).write_u32(1);
    gicc_write(GICC_PMR, 0xFF);
    gicc_write(GICC_CTLR, 1);
}

/// Enable a per-CPU interrupt (PPI/SGI), e.g. the generic timer.
pub fn enable_ppi(intid: u32) {
    enable_intid(intid);
}

/// Enable a shared peripheral interrupt (SPI) and route it to CPU 0.
pub fn enable_spi(intid: u32) {
    enable_intid(intid);
    // ITARGETSR is byte-per-interrupt; target CPU 0.
    kor::PhysAddr::new(GICD_BASE + GICD_ITARGETSR + intid as usize).to_ka().write_u8(0x01);
}
