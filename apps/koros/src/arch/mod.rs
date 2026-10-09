//! Arch-specific application code: per-arch boot modules + platform constants.
//!
//! The boot trampolines live in the per-arch `boot.rs` modules, composed
//! from kor-arch fragment macros with this app's mapping policy inline.

use kor::config::{Console as ConsoleCfg, PlatformConfig};
#[cfg(target_arch = "aarch64")]
mod aarch64;
#[cfg(target_arch = "riscv64")]
mod riscv64;
#[cfg(target_arch = "x86_64")]
mod x86_64;

/// Board / platform configuration for the QEMU targets.
pub(crate) fn platform_config() -> PlatformConfig {
    #[cfg(target_arch = "riscv64")]
    return PlatformConfig {
        console: ConsoleCfg::Ns16550aMmio { base: 0x1000_0000 },
        firmware_phys_start: 0x8000_0000,
        dtb: 0, // passed by OpenSBI in a1
        pci: None,
    };
    #[cfg(target_arch = "aarch64")]
    return PlatformConfig {
        console: ConsoleCfg::Pl011Mmio { base: 0x0900_0000 },
        firmware_phys_start: 0x4000_0000,
        dtb: 0, // passed in x0
        pci: None,
    };
    #[cfg(target_arch = "x86_64")]
    return PlatformConfig {
        console: ConsoleCfg::Ns16550aPort { base: 0x3F8 },
        firmware_phys_start: 0,
        dtb: 0,
        pci: None,
    };
}
