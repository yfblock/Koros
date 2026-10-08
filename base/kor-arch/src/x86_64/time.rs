//! x86_64 timer: the Local APIC timer in periodic mode.
//!
//! The APIC timer frequency isn't architecturally fixed, so we calibrate it
//! once against the 8254 PIT (channel 2, gated so it needs no interrupt), then
//! run it periodically at [`kor::time::TICK_HZ`].

use kor::arch as mm;
use spin::Once;
use x86_64::instructions::port::Port;

/// APIC timer initial count for one tick, calibrated once and shared by all
/// CPUs (the timer frequency is identical across cores).  Calibration uses the
/// shared PIT, so it must run on only one CPU at a time — `Once` guarantees it.
static TIMER_COUNT: Once<u32> = Once::new();

const APIC_BASE_PHYS: usize = 0xFEE0_0000;
const APIC_EOI: usize = 0xB0;
const APIC_SVR: usize = 0xF0;
const APIC_LVT_TIMER: usize = 0x320;
const APIC_TIMER_INITCNT: usize = 0x380;
const APIC_TIMER_CURCNT: usize = 0x390;
const APIC_TIMER_DIV: usize = 0x3E0;

/// IDT vector for the APIC timer (above the 0-31 exception range).
const TIMER_VECTOR: u32 = 0x20;
const LVT_PERIODIC: u32 = 1 << 17;
const LVT_MASKED: u32 = 1 << 16;
const TIMER_DIV_16: u32 = 0x3;

/// PIT input clock (Hz) and the count for a 10 ms calibration window.
const PIT_HZ: u32 = 1_193_182;
const PIT_CAL_MS: u32 = 10;
const PIT_CAL_COUNT: u16 = (PIT_HZ / (1000 / PIT_CAL_MS)) as u16;

fn lapic_read(off: usize) -> u32 {
    // SAFETY: LAPIC MMIO is mapped in the direct map.
    unsafe { ((mm::phys_to_virt(APIC_BASE_PHYS) + off) as *const u32).read_volatile() }
}

fn lapic_write(off: usize, val: u32) {
    // SAFETY: LAPIC MMIO is mapped in the direct map.
    unsafe { ((mm::phys_to_virt(APIC_BASE_PHYS) + off) as *mut u32).write_volatile(val) }
}

/// Busy-wait ~10 ms using PIT channel 2 (gated via port 0x61, no interrupt).
fn pit_wait_10ms() {
    // SAFETY: legacy PIT / control port access.
    unsafe {
        let mut port61 = Port::<u8>::new(0x61);
        let mut cmd = Port::<u8>::new(0x43);
        let mut ch2 = Port::<u8>::new(0x42);

        // Speaker off (bit1=0), gate low (bit0=0) to reset.
        let base = port61.read() & 0xFC;
        port61.write(base);
        // Channel 2, lobyte/hibyte, mode 0 (terminal count), binary.
        cmd.write(0xB0);
        ch2.write((PIT_CAL_COUNT & 0xFF) as u8);
        ch2.write((PIT_CAL_COUNT >> 8) as u8);
        // Raise the gate to start counting.
        port61.write(base | 0x01);
        // Wait until OUT (bit 5) goes high at terminal count.
        while port61.read() & 0x20 == 0 {
            core::hint::spin_loop();
        }
    }
}

/// Calibrate: count APIC timer ticks during a 10 ms PIT window; scale to the
/// desired tick period.
fn calibrate() -> u32 {
    lapic_write(APIC_TIMER_DIV, TIMER_DIV_16);
    lapic_write(APIC_LVT_TIMER, LVT_MASKED); // no interrupts during calibration
    lapic_write(APIC_TIMER_INITCNT, 0xFFFF_FFFF);

    pit_wait_10ms();

    let elapsed = 0xFFFF_FFFFu32 - lapic_read(APIC_TIMER_CURCNT);
    // `elapsed` is ticks per 10 ms; convert to ticks per timer period.
    let per_sec = elapsed as u64 * (1000 / PIT_CAL_MS as u64);
    (per_sec / kor::time::TICK_HZ).max(1) as u32
}

/// Enable the LAPIC and start the periodic timer; enable interrupts.
pub fn init() {
    // Software-enable the LAPIC (spurious vector 0xFF).
    lapic_write(APIC_SVR, lapic_read(APIC_SVR) | 0x100 | 0xFF);

    // Calibrate once (boot CPU); secondary CPUs reuse the stored count.
    let count = *TIMER_COUNT.call_once(calibrate);
    lapic_write(APIC_TIMER_DIV, TIMER_DIV_16);
    lapic_write(APIC_LVT_TIMER, TIMER_VECTOR | LVT_PERIODIC);
    lapic_write(APIC_TIMER_INITCNT, count);

    // Mask the legacy 8259 PIC — otherwise its IRQ0 (PIT) is delivered on
    // vector 0x08 (the double-fault vector).  We use the APIC timer only.
    // SAFETY: legacy PIC data-port writes.
    unsafe {
        Port::<u8>::new(0x21).write(0xFF);
        Port::<u8>::new(0xA1).write(0xFF);
    }
    // Global interrupts are enabled separately via `kor::irq::enable`.
}

/// Acknowledge the timer interrupt (signal end-of-interrupt to the LAPIC).
pub fn handle_tick() {
    lapic_write(APIC_EOI, 0);
}
