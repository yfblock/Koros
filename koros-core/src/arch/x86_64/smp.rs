//! x86_64 SMP: Local APIC identity + AP bring-up via INIT-SIPI-SIPI.
//!
//! Each application processor starts in 16-bit real mode at a low physical
//! page (`AP_BOOT_PADDR`); the `ap_boot.S` trampoline copied there walks it up
//! to long mode on the kernel page table and jumps to [`ap_rust_entry`].  APs
//! are brought up one at a time so they can share the single trampoline page.

use crate::mm;
use crate::smp::MAX_CPUS;

core::arch::global_asm!(include_str!("ap_boot.S"));

/// Physical page (< 1 MiB) holding the AP trampoline; SIPI vector = page number.
const AP_BOOT_PADDR: usize = 0x8000;
const SLOT_CR3: usize = AP_BOOT_PADDR + 0xfe0;
const SLOT_STACK: usize = AP_BOOT_PADDR + 0xff0;
const SLOT_ENTRY: usize = AP_BOOT_PADDR + 0xff8;

/// Local APIC MMIO base (xAPIC).
const APIC_BASE_PHYS: usize = 0xFEE0_0000;
const APIC_ID: usize = 0x20;
const APIC_SVR: usize = 0xF0;
const APIC_ICR_LO: usize = 0x300;
const APIC_ICR_HI: usize = 0x310;
const ICR_DELIVERY_PENDING: u32 = 1 << 12;

/// Per-secondary-CPU kernel stack size.
const SECONDARY_STACK_SIZE: usize = 0x1_0000; // 64 KiB

#[repr(align(16))]
struct Stack([u8; SECONDARY_STACK_SIZE]);

static mut SECONDARY_STACKS: [Stack; MAX_CPUS] =
    [const { Stack([0; SECONDARY_STACK_SIZE]) }; MAX_CPUS];

unsafe extern "C" {
    fn ap_start();
    fn ap_end();
}

fn lapic_reg(off: usize) -> *mut u32 {
    (mm::phys_to_virt(APIC_BASE_PHYS) + off) as *mut u32
}

fn lapic_read(off: usize) -> u32 {
    // SAFETY: the LAPIC MMIO region is identity/high-half mapped.
    unsafe { lapic_reg(off).read_volatile() }
}

fn lapic_write(off: usize, val: u32) {
    // SAFETY: the LAPIC MMIO region is identity/high-half mapped.
    unsafe { lapic_reg(off).write_volatile(val) }
}

/// Software-enable this CPU's Local APIC (spurious vector 0xFF, enable bit).
fn enable_lapic() {
    lapic_write(APIC_SVR, lapic_read(APIC_SVR) | 0x100 | 0xFF);
}

/// Local APIC id of the current CPU.
pub fn cpu_id() -> usize {
    (lapic_read(APIC_ID) >> 24) as usize
}

/// No-op: the CPU id is read from hardware.
pub fn set_cpu_id(_id: usize) {}

/// Idle until an interrupt.
pub fn wait_for_interrupt() {
    // SAFETY: `hlt` halts until the next interrupt.
    unsafe { core::arch::asm!("hlt") };
}

fn delay(iters: u64) {
    for _ in 0..iters {
        core::hint::spin_loop();
    }
}

fn icr_wait(apic: u32, low: u32) {
    lapic_write(APIC_ICR_HI, apic << 24);
    lapic_write(APIC_ICR_LO, low);
    while lapic_read(APIC_ICR_LO) & ICR_DELIVERY_PENDING != 0 {
        core::hint::spin_loop();
    }
}

/// Bring up the application processors.  Returns the number that came online.
pub fn start_secondaries() -> usize {
    enable_lapic();
    let boot = cpu_id();

    // Copy the trampoline into the low boot page and record CR3 + entry.
    let len = ap_end as *const () as usize - ap_start as *const () as usize;
    let cr3: usize;
    // SAFETY: read the active PML4 base.
    unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3) };
    // SAFETY: `AP_BOOT_PADDR` is a free low page (x86 excludes <1MiB from the
    // frame allocator); the direct map makes it writable here.
    unsafe {
        core::ptr::copy_nonoverlapping(
            ap_start as *const u8,
            mm::phys_to_virt(AP_BOOT_PADDR) as *mut u8,
            len,
        );
        (mm::phys_to_virt(SLOT_CR3) as *mut usize).write_volatile(cr3);
        (mm::phys_to_virt(SLOT_ENTRY) as *mut usize).write_volatile(ap_rust_entry as usize);
    }

    let vector = (AP_BOOT_PADDR >> 12) as u32;
    let mut started = 0;
    for apic in 0..MAX_CPUS {
        if apic == boot {
            continue;
        }
        // Point the trampoline at this AP's stack.
        let sp = unsafe {
            core::ptr::addr_of!(SECONDARY_STACKS[apic]) as usize + SECONDARY_STACK_SIZE
        };
        // SAFETY: writing the trampoline's stack slot before the SIPI.
        unsafe { (mm::phys_to_virt(SLOT_STACK) as *mut usize).write_volatile(sp) };

        let before = crate::smp::online_count();
        // INIT, then two STARTUP IPIs (per the Intel MP protocol).
        icr_wait(apic as u32, 0x0000_4500);
        delay(1_000_000);
        icr_wait(apic as u32, 0x0000_4600 | vector);
        delay(200_000);
        icr_wait(apic as u32, 0x0000_4600 | vector);

        // Wait (bounded) for this AP to register before reusing the page.
        let mut spins = 0u64;
        while crate::smp::online_count() == before && spins < 300_000_000 {
            core::hint::spin_loop();
            spins += 1;
        }
        if crate::smp::online_count() > before {
            started += 1;
        } else {
            // APIC ids are contiguous; a non-responding id means no more CPUs.
            break;
        }
    }
    started
}

/// 64-bit Rust entry for an application processor, reached from the trampoline
/// once it is in long mode on the kernel page table with its stack set.
#[unsafe(no_mangle)]
extern "C" fn ap_rust_entry() -> ! {
    enable_lapic();
    crate::smp::secondary_entry(cpu_id())
}
