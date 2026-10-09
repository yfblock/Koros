//! `InterruptController` stub for x86_64.
//!
//! x86_64 routes device IRQs through the IDT (and PCI devices via MSI/legacy
//! INTx handled in PCI config space), not through a discoverable external-IRQ
//! controller reached from the trap handler.  Virtio-blk therefore runs in
//! polling mode on this architecture.  The stub lets the shared virtio path
//! compile uniformly and no-ops at runtime.




pub fn init() {
    // LAPIC enable is part of IDT / trap setup; nothing controller-global.
}

pub fn enable_device_irq(_interrupts: &[u32; 3]) -> Option<u32> {
    None
}

pub fn dispatch_external(_handle_irq: &mut dyn FnMut(u32), _timer_tick: &mut dyn FnMut()) {
    // x86_64 external IRQs go through IDT vectors, not a controller dispatch.
}

// --- Local IRQ mask (DAIF.I / sie) ---

use core::arch::asm;

const RFLAGS_IF: u64 = 1 << 9;

pub fn enable() {
    // SAFETY: sets the interrupt flag.
unsafe { asm!("sti", options(nomem, nostack)) };
}

pub fn disable() {
    // SAFETY: clears the interrupt flag.
unsafe { asm!("cli", options(nomem, nostack)) };
}

pub fn is_enabled() -> bool {
let flags: u64;
    // SAFETY: reads RFLAGS.
unsafe { asm!("pushfq; pop {}", out(reg) flags, options(nomem)) };
flags & RFLAGS_IF != 0
}
