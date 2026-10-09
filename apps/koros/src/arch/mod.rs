//! Arch-specific application code: per-arch boot modules + platform constants.
//!
//! The boot trampolines live in the per-arch `boot.rs` modules, composed
//! from kor-arch fragment macros with this app's mapping policy inline.

use kor::config::PlatformConfig;
#[cfg(target_arch = "aarch64")]
mod aarch64;
#[cfg(target_arch = "aarch64")]
pub use aarch64::trap;
#[cfg(target_arch = "riscv64")]
mod riscv64;
#[cfg(target_arch = "riscv64")]
pub use riscv64::trap;
#[cfg(target_arch = "x86_64")]
mod x86_64;
#[cfg(target_arch = "x86_64")]
pub use x86_64::trap;

/// Board / platform configuration for the QEMU targets.
pub(crate) fn platform_config() -> PlatformConfig {
    #[cfg(target_arch = "riscv64")]
    return PlatformConfig {
        firmware_phys_start: 0x8000_0000,
        dtb: 0, // passed by OpenSBI in a1
        pci: None,
    };
    #[cfg(target_arch = "aarch64")]
    return PlatformConfig {
        firmware_phys_start: 0x4000_0000,
        dtb: 0, // passed in x0
        pci: None,
    };
    #[cfg(target_arch = "x86_64")]
    return PlatformConfig {
        firmware_phys_start: 0,
        dtb: 0,
        pci: None,
    };
}
