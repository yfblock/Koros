//! Arch-specific application code: boot MMU policy + platform constants.
#[cfg(target_arch = "loongarch64")]
use kor::config::PciEcam;
use kor::config::{Console as ConsoleCfg, PlatformConfig};

kor_arch::boot_stack!(0x80000);

unsafe extern "C" {
    static bstack_top: u64;
    #[cfg(target_arch = "riscv64")]
    fn rust_entry();
    #[cfg(target_arch = "x86_64")]
    fn rust_entry();
    #[cfg(target_arch = "aarch64")]
    fn rust_entry();
    #[cfg(target_arch = "loongarch64")]
    fn rust_entry();
}

#[cfg(target_arch = "riscv64")]
kor_arch::boot_trampoline_riscv64!(bstack_top, rust_entry);
#[cfg(target_arch = "x86_64")]
kor_arch::boot_trampoline_x86_64!(bstack_top, rust_entry);
#[cfg(target_arch = "aarch64")]
kor_arch::boot_trampoline_aarch64!(bstack_top, rust_entry);
#[cfg(target_arch = "loongarch64")]
kor_arch::boot_trampoline_loongarch64!(bstack_top, rust_entry);

#[cfg(target_arch = "aarch64")]
mod aarch64;
#[cfg(target_arch = "loongarch64")]
mod loongarch64;
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
    #[cfg(target_arch = "loongarch64")]
    return PlatformConfig {
        console: ConsoleCfg::Ns16550aMmio { base: 0x1FE0_01E0 },
        firmware_phys_start: 0x8000_0000,
        dtb: 0x100000, // fixed QEMU address (no register-passed pointer)
        pci: Some(PciEcam {
            ecam_base: 0x2000_0000,
            mmio_base: 0x4000_0000,
            mmio_size: 0x4000_0000,
        }),
    };
    #[cfg(target_arch = "x86_64")]
    return PlatformConfig {
        console: ConsoleCfg::Ns16550aPort { base: 0x3F8 },
        firmware_phys_start: 0,
        dtb: 0,
        pci: None,
    };
}
