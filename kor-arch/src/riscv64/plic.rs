//! riscv64 PLIC (Platform-Level Interrupt Controller) — QEMU `virt`.
//!
//! Routes device interrupts to a hart's supervisor external-interrupt line.
//! Only the hart that calls [`enable`]/[`enable_seie`] receives them; its
//! handler wakes any waiting task via the driver's completion signal, so the
//! waiter can run on any CPU.

use core::arch::asm;

/// QEMU `virt` PLIC base.
const PLIC_BASE: usize = 0x0c00_0000;

/// Supervisor-external interrupt-enable bit in `sie`.
const SIE_SEIE: usize = 1 << 9;

/// S-mode context index for hart `h` (M-mode is `2h`, S-mode is `2h+1`).
pub fn context(hart: usize) -> usize {
    2 * hart + 1
}

fn reg(off: usize) -> *mut u32 {
    (kor::arch::phys_to_virt(PLIC_BASE) + off) as *mut u32
}

/// Claim register for S-mode context `ctx` (read = claim, write = complete).
pub fn claim_reg(ctx: usize) -> *mut u32 {
    reg(0x20_0004 + ctx * 0x1000)
}

/// Enable interrupt source `irq`, routed to hart `hart`'s S-mode context.
pub fn enable(irq: u32, hart: usize) {
    let ctx = context(hart);
    // SAFETY: PLIC MMIO is mapped in the direct map.
    unsafe {
        reg(irq as usize * 4).write_volatile(1); // priority = 1 (0 disables)
        let en = reg(0x2000 + ctx * 0x80 + (irq as usize / 32) * 4);
        en.write_volatile(en.read_volatile() | (1 << (irq % 32)));
        reg(0x20_0000 + ctx * 0x1000).write_volatile(0); // threshold 0
    }
}

/// Enable the supervisor external-interrupt line on the current hart.
pub fn enable_seie() {
    // SAFETY: sets the SEIE bit in `sie`.
    unsafe { asm!("csrs sie, {}", in(reg) SIE_SEIE) };
}

