//! aarch64 timer: the EL1 physical generic timer (CNTP), routed through a
//! GICv2 interrupt controller (QEMU `virt`).

use core::sync::atomic::{AtomicU64, Ordering};

use crate::mm;

// GICv2 register bases (QEMU `virt`).
const GICD_BASE: usize = 0x0800_0000;
const GICC_BASE: usize = 0x0801_0000;

const GICD_CTLR: usize = 0x000;
const GICD_ISENABLER: usize = 0x100;
const GICD_IPRIORITYR: usize = 0x400;

const GICC_CTLR: usize = 0x000;
const GICC_PMR: usize = 0x004;
const GICC_IAR: usize = 0x00C;
const GICC_EOIR: usize = 0x010;

/// EL1 non-secure physical timer PPI (PPI 14 -> INTID 30).
const TIMER_INTID: u32 = 30;

/// Countdown value programmed each period (set in `init`).
static INTERVAL: AtomicU64 = AtomicU64::new(0);

fn gicd_write(off: usize, val: u32) {
    // SAFETY: GIC distributor MMIO is mapped in the direct map.
    unsafe { ((mm::phys_to_virt(GICD_BASE) + off) as *mut u32).write_volatile(val) };
}

fn gicc_read(off: usize) -> u32 {
    // SAFETY: GIC CPU-interface MMIO is mapped in the direct map.
    unsafe { ((mm::phys_to_virt(GICC_BASE) + off) as *const u32).read_volatile() }
}

fn gicc_write(off: usize, val: u32) {
    // SAFETY: GIC CPU-interface MMIO is mapped in the direct map.
    unsafe { ((mm::phys_to_virt(GICC_BASE) + off) as *mut u32).write_volatile(val) };
}

fn cntfrq() -> u64 {
    let f: u64;
    // SAFETY: reads the read-only counter-frequency register.
    unsafe { core::arch::asm!("mrs {}, cntfrq_el0", out(reg) f) };
    f
}

fn set_timer(interval: u64) {
    // SAFETY: programs the EL1 physical timer downcounter and enables it.
    unsafe {
        core::arch::asm!("msr cntp_tval_el0, {}", in(reg) interval);
        core::arch::asm!("msr cntp_ctl_el0, {}", in(reg) 1u64); // ENABLE, unmasked
    }
}

/// Initialise the GICv2, program the timer, and unmask IRQs.
pub fn init() {
    // GIC distributor: enable, route + enable the timer INTID.
    gicd_write(GICD_CTLR, 1);
    gicd_write(GICD_IPRIORITYR + TIMER_INTID as usize, 0); // highest priority (byte)
    gicd_write(GICD_ISENABLER + (TIMER_INTID as usize / 32) * 4, 1 << (TIMER_INTID % 32));

    // GIC CPU interface: allow all priorities, enable.
    gicc_write(GICC_PMR, 0xFF);
    gicc_write(GICC_CTLR, 1);

    let interval = cntfrq() / crate::time::TICK_HZ;
    INTERVAL.store(interval, Ordering::Relaxed);
    set_timer(interval);
    // Global interrupts are enabled separately via `crate::irq::enable`.
}

/// Acknowledge the timer interrupt at the GIC and reprogram the next period.
pub fn handle_tick() {
    let iar = gicc_read(GICC_IAR);
    set_timer(INTERVAL.load(Ordering::Relaxed));
    gicc_write(GICC_EOIR, iar);
}
