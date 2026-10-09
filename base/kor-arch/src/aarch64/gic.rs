//! aarch64 GICv2 interrupt controller (QEMU `virt`).
//!
//! Handles the generic-timer PPI and device SPIs (e.g. virtio-mmio).  The IRQ
//! trap path calls [`dispatch`], which reads the acknowledge register, routes
//! the timer to the tick path and everything else to the external-interrupt
//! registry, then signals end-of-interrupt.


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
    kor::PhysAddr::new(GICD_BASE + off).to_va()
}

fn gicc(off: usize) -> kor::KernelAddr {
    kor::PhysAddr::new(GICC_BASE + off).to_va()
}

fn gicc_read(off: usize) -> u32 {
    // SAFETY: GIC CPU-interface MMIO is mapped in the direct map.
    gicc(off).read::<u32>()
}

fn gicc_write(off: usize, val: u32) {
    // SAFETY: GIC CPU-interface MMIO is mapped in the direct map.
    gicc(off).write::<u32>(val);
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
    kor::PhysAddr::new(GICD_BASE + GICD_IPRIORITYR + intid as usize).to_va().write::<u8>(0);
    let en = gicd(GICD_ISENABLER + (intid as usize / 32) * 4);
    en.write::<u32>(en.read::<u32>() | (1 << (intid % 32)));
}

/// Distributor + CPU-interface enable and priority mask.  Called per CPU.
pub fn init() {
    // SAFETY: GIC MMIO is mapped.
    gicd(GICD_CTLR).write::<u32>(1);
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
    kor::PhysAddr::new(GICD_BASE + GICD_ITARGETSR + intid as usize).to_va().write::<u8>(0x01);
}
