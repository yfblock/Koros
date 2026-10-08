//! Arch-specific application code: boot MMU policy + platform constants.
use kor::config::{self, PlatformConfig};

kor_arch::boot_stack!(0x80000);

unsafe extern "C" {
    static bstack_top: u64;
    #[cfg(target_arch = "riscv64")]
    fn rust_entry();
    #[cfg(target_arch = "x86_64")]
    fn rust_entry();
    #[cfg(target_arch = "aarch64")]
    fn rust_entry();
}

#[cfg(target_arch = "riscv64")]
kor_arch::boot_trampoline_riscv64!(bstack_top, rust_entry);
#[cfg(target_arch = "x86_64")]
kor_arch::boot_trampoline_x86_64!(bstack_top, rust_entry);
#[cfg(target_arch = "aarch64")]
kor_arch::boot_trampoline_aarch64!(bstack_top, rust_entry);

#[cfg(target_arch = "riscv64")]
mod boot_mmu;

pub(crate) fn platform_config() -> PlatformConfig {
    #[cfg(target_arch = "riscv64")]
    return PlatformConfig {
        console: config::Console::Ns16550aMmio { base: 0x1000_0000 },
        firmware_phys_start: 0x8000_0000,
        dtb: 0,
        pci: None,
    };
    #[cfg(target_arch = "aarch64")]
    return PlatformConfig {
        console: config::Console::Pl011Mmio { base: 0x0900_0000 },
        firmware_phys_start: 0x4000_0000,
        dtb: 0,
        pci: None,
    };
    #[cfg(target_arch = "x86_64")]
    return PlatformConfig {
        console: config::Console::Ns16550aPort { base: 0x3F8 },
        firmware_phys_start: 0,
        dtb: 0,
        pci: None,
    };
    #[cfg(not(any(
        target_arch = "riscv64",
        target_arch = "aarch64",
        target_arch = "x86_64"
    )))]
    compile_error!("korix supports riscv64, aarch64, and x86_64");
}

#[cfg(target_arch = "aarch64")]
mod boot_aarch64;
#[cfg(target_arch = "riscv64")]
mod boot_riscv64;
#[cfg(target_arch = "x86_64")]
mod boot_x86_64;
