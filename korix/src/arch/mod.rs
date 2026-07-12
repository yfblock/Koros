//! Architecture abstraction layer for korix.
//!
//! Reuses kor-arch for boot/page-table/IRQ/timer, and adds:
//! - User-mode (EL0) trap handling for syscalls
//! - Per-arch user space context switching
//! - Device probing (FDT + PCI)

pub mod trap;
pub mod user_ctx;

/// Re-export console from kor-arch.
pub use kor_arch::console;

/// Dispatch an external IRQ to the appropriate device handler.
pub fn dispatch_irq(irq: u32) {
    let _ = irq;
}

/// Probe devices via FDT (riscv64/aarch64) or PCI (x86_64).
pub fn probe_devices() {
    #[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
    {
        let dtb = kor::arch::current().dtb_ptr();
        if dtb != 0 {
            kor::println!("[korix] probing FDT at {:#x}", dtb);
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        kor::println!("[korix] PCI probe (TODO)");
    }
}
