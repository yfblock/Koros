//! aarch64 GICv2 interrupt controller (QEMU `virt`).
//!
//! Handles the generic-timer PPI and device SPIs (e.g. virtio-mmio).  The IRQ
//! trap path calls [`dispatch`], which reads the acknowledge register, routes
//! the timer to the tick path and everything else to the external-interrupt
//! registry, then signals end-of-interrupt.

use crate::mm;

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

/// Spurious interrupt id (1020-1023).
const SPURIOUS: u32 = 1020;

fn gicd(off: usize) -> *mut u32 {
    (mm::phys_to_virt(GICD_BASE) + off) as *mut u32
}

fn gicc(off: usize) -> *mut u32 {
    (mm::phys_to_virt(GICC_BASE) + off) as *mut u32
}

fn gicc_read(off: usize) -> u32 {
    // SAFETY: GIC CPU-interface MMIO is mapped in the direct map.
    unsafe { gicc(off).read_volatile() }
}

fn gicc_write(off: usize, val: u32) {
    // SAFETY: GIC CPU-interface MMIO is mapped in the direct map.
    unsafe { gicc(off).write_volatile(val) };
}

/// Enable an interrupt id (sets priority to 0 = highest, sets the enable bit).
fn enable_intid(intid: u32) {
    // SAFETY: GIC distributor MMIO is mapped in the direct map.
    unsafe {
        (gicd(GICD_IPRIORITYR) as *mut u8)
            .add(intid as usize)
            .write_volatile(0);
        let en = gicd(GICD_ISENABLER + (intid as usize / 32) * 4);
        en.write_volatile(en.read_volatile() | (1 << (intid % 32)));
    }
}

/// Distributor + CPU-interface enable and priority mask.  Called per CPU.
pub fn init() {
    // SAFETY: GIC MMIO is mapped.
    unsafe { gicd(GICD_CTLR).write_volatile(1) };
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
    // SAFETY: ITARGETSR is byte-per-interrupt; target CPU 0.
    unsafe {
        (gicd(GICD_ITARGETSR) as *mut u8)
            .add(intid as usize)
            .write_volatile(0x01);
    }
}

/// Handle an IRQ: acknowledge, dispatch (timer vs device), signal EOI.
pub fn dispatch() {
    let iar = gicc_read(GICC_IAR);
    let intid = iar & 0x3ff;
    if intid >= SPURIOUS {
        return;
    }
    if intid == super::time::TIMER_INTID {
        crate::time::tick(); // reprograms the timer and counts the tick
        gicc_write(GICC_EOIR, iar);
        crate::sched::preempt();
    } else {
        crate::drivers::irq::handle(intid);
        gicc_write(GICC_EOIR, iar);
    }
}
