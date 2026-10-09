//! x86_64 SMP: Local APIC identity + AP bring-up via INIT-SIPI-SIPI.
//!
//! Each application processor starts in 16-bit real mode at a low physical
//! page (`AP_BOOT_PADDR`); the `ap_boot.S` trampoline copied there walks it up
//! to long mode on the kernel page table and jumps to [`ap_rust_entry`].  APs
//! are brought up one at a time so they can share the single trampoline page.

use kor::smp::MAX_CPUS;

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

fn lapic_reg(off: usize) -> kor::mmio::Reg<u32> {
    kor::mmio::Reg::at_pa(APIC_BASE_PHYS + off)
}

fn lapic_read(off: usize) -> u32 {
    lapic_reg(off).read()
}

fn lapic_write(off: usize, val: u32) {
    lapic_reg(off).write(val)
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

/// QEMU ACPI PM1a control register (PIIX4/q35 default base).
const QEMU_PM1A_CNT: u16 = 0x604;
/// Legacy QEMU poweroff port (older machine models / bochs).
const QEMU_PM1A_CNT_LEGACY: u16 = 0xB004;
/// `SLP_EN` with `SLP_TYP=0` (S5) requests a poweroff.
const PM1_SLP_EN: u16 = 1 << 13;

/// Power the machine off via the QEMU ACPI PM1a register, then park.
pub fn shutdown() -> ! {
    // SAFETY: port I/O writes to the PM registers; halting is always safe.
    unsafe {
        core::arch::asm!(
            "out dx, ax",
            in("dx") QEMU_PM1A_CNT,
            in("ax") PM1_SLP_EN,
            options(nomem, nostack, preserves_flags)
        );
        core::arch::asm!(
            "out dx, ax",
            in("dx") QEMU_PM1A_CNT_LEGACY,
            in("ax") PM1_SLP_EN,
            options(nomem, nostack, preserves_flags)
        );
    }
    loop {
        // SAFETY: `hlt` halts until the next interrupt.
        unsafe { core::arch::asm!("hlt") };
    }
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
            kor::PhysAddr::new(AP_BOOT_PADDR).to_va().raw() as *mut u8,
            len,
        );
        kor::mmio::Reg::<usize>::at_pa(SLOT_CR3).write(cr3);
        kor::mmio::Reg::<usize>::at_pa(SLOT_ENTRY).write(ap_rust_entry as *const () as usize);
    }

    let vector = (AP_BOOT_PADDR >> 12) as u32;
    let mut started = 0;
    for apic in 0..MAX_CPUS {
        if apic == boot {
            continue;
        }
        // Point the trampoline at this AP's stack.
        let sp = unsafe {
            core::ptr::addr_of!(SECONDARY_STACKS[apic].0) as usize + SECONDARY_STACK_SIZE
        };
        // SAFETY: writing the trampoline's stack slot before the SIPI.
        kor::mmio::Reg::<usize>::at_pa(SLOT_STACK).write(sp);

        let before = kor::smp::online_count();
        // INIT, then two STARTUP IPIs (per the Intel MP protocol).
        icr_wait(apic as u32, 0x0000_4500);
        delay(1_000_000);
        icr_wait(apic as u32, 0x0000_4600 | vector);
        delay(200_000);
        icr_wait(apic as u32, 0x0000_4600 | vector);

        // Wait (bounded) for this AP to register before reusing the page.
        let mut spins = 0u64;
        while kor::smp::online_count() == before && spins < 300_000_000 {
            core::hint::spin_loop();
            spins += 1;
        }
        if kor::smp::online_count() > before {
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
    unsafe { crate::secondary_entry(cpu_id()) }
}
