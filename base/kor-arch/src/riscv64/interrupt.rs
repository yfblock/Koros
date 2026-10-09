//! `InterruptController` implementation for riscv64 — the QEMU `virt` PLIC.
//!
//! The PLIC has no global one-shot init (priority/enables are per-source,
//! SEIE is per-hart), so [`PlicController::init`] is a no-op; device IRQs are
//! armed one-by-one via [`crate::enable_device_irq`] and dispatched by
//! [`dispatch_external`], which claims/ completes each pending source.


use core::arch::asm;



pub fn init() {
    // No global PLIC init; SEIE is enabled per-hart in `enable_device_irq`.
}

pub fn enable_device_irq(interrupts: &[u32; 3]) -> Option<u32> {
    // PLIC uses a single `interrupts` cell: the source number.
    let irq = interrupts[0];
    if irq == 0 {
        return None;
    }
    let hart = super::smp::cpu_id();
    enable_source(irq, hart);
    enable_seie();
    kor::println!("virtio-blk: IRQ {} via PLIC on hart {}", irq, hart);
    Some(irq)
}

pub fn dispatch_external(handle_irq: &mut dyn FnMut(u32), _timer_tick: &mut dyn FnMut()) {
    let ctx = context(super::smp::cpu_id());
    let claim = claim_reg(ctx);
    loop {
        // SAFETY: claim register read returns the highest-priority pending IRQ.
        let irq = claim.read::<u32>();
        if irq == 0 {
            break;
        }
        handle_irq(irq);
        // SAFETY: writing the IRQ back signals completion.
        claim.write::<u32>(irq);
    }
}

// --- Local IRQ mask (DAIF.I / sie) ---


const SSTATUS_SIE: usize = 1 << 1;

pub fn enable() {
    // SAFETY: sets the supervisor interrupt-enable bit.
unsafe { asm!("csrs sstatus, {}", in(reg) SSTATUS_SIE) };
}

pub fn disable() {
    // SAFETY: clears the supervisor interrupt-enable bit.
unsafe { asm!("csrc sstatus, {}", in(reg) SSTATUS_SIE) };
}

pub fn is_enabled() -> bool {
let sstatus: usize;
    // SAFETY: reads a status CSR.
unsafe { asm!("csrr {}, sstatus", out(reg) sstatus) };
sstatus & SSTATUS_SIE != 0
}


// --- PLIC registers ---


/// QEMU `virt` PLIC base.
const PLIC_BASE: usize = 0x0c00_0000;

/// Supervisor-external interrupt-enable bit in `sie`.
const SIE_SEIE: usize = 1 << 9;

/// S-mode context index for hart `h` (M-mode is `2h`, S-mode is `2h+1`).
pub fn context(hart: usize) -> usize {
2 * hart + 1
}

use kor::KernelAddr;

fn reg(off: usize) -> KernelAddr {
kor::PhysAddr::new(PLIC_BASE + off).to_ka()
}

/// Claim register for S-mode context `ctx` (read = claim, write = complete).
pub fn claim_reg(ctx: usize) -> KernelAddr {
reg(0x20_0004 + ctx * 0x1000)
}

/// Enable interrupt source `irq`, routed to hart `hart`'s S-mode context.
pub fn enable_source(irq: u32, hart: usize) {
let ctx = context(hart);
    // SAFETY: PLIC MMIO is mapped in the direct map.
    reg(irq as usize * 4).write_u32(1); // priority = 1 (0 disables)
    let en = reg(0x2000 + ctx * 0x80 + (irq as usize / 32) * 4);
    en.write_u32(en.read_u32() | (1 << (irq % 32)));
    reg(0x20_0000 + ctx * 0x1000).write_u32(0); // threshold 0

}

/// Enable the supervisor external-interrupt line on the current hart.
pub fn enable_seie() {
    // SAFETY: sets the SEIE bit in `sie`.
unsafe { asm!("csrs sie, {}", in(reg) SIE_SEIE) };
}
