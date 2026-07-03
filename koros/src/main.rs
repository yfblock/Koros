#![no_std]
#![no_main]

use koros_core::platform::{self, Console, PlatformConfig};
#[cfg(target_arch = "loongarch64")]
use koros_core::platform::PciEcam;
use koros_core::{cmdline, mm, trap};

/// Board / platform configuration for the QEMU `virt` (and x86 `q35`) targets.
///
/// This is the single place that holds the concrete hardware addresses; it is
/// handed to `koros_core` at boot so the library stays board-agnostic.
fn platform_config() -> PlatformConfig {
    #[cfg(target_arch = "riscv64")]
    return PlatformConfig {
        console: Console::Ns16550aMmio { base: 0x1000_0000 },
        firmware_phys_start: 0x8000_0000,
        dtb: 0, // passed by OpenSBI in a1
        pci: None,
    };
    #[cfg(target_arch = "aarch64")]
    return PlatformConfig {
        console: Console::Pl011Mmio { base: 0x0900_0000 },
        firmware_phys_start: 0x4000_0000,
        dtb: 0, // passed in x0
        pci: None,
    };
    #[cfg(target_arch = "loongarch64")]
    return PlatformConfig {
        console: Console::Ns16550aMmio { base: 0x1FE0_01E0 },
        firmware_phys_start: 0x8000_0000,
        dtb: 0x100000, // fixed QEMU address (no register-passed pointer)
        // QEMU loongarch `virt` puts virtio on PCIe (ECAM), not virtio-mmio.
        pci: Some(PciEcam {
            ecam_base: 0x2000_0000,
            mmio_base: 0x4000_0000,
            mmio_size: 0x4000_0000,
        }),
    };
    #[cfg(target_arch = "x86_64")]
    return PlatformConfig {
        console: Console::Ns16550aPort { base: 0x3F8 },
        firmware_phys_start: 0,
        dtb: 0,
        pci: None,
    };
}

/// Kernel entry point, called by the architecture boot code
/// (`koros_core::arch::<arch>::boot::rust_entry`) after early setup.
#[unsafe(no_mangle)]
extern "C" fn kernel_main() -> ! {
    // Install the board configuration first, before any console output or
    // memory setup reads it back.
    platform::init(platform_config());
    trap::init();
    mm::init(); // captures the boot command line early (see mm::init)
    koros_core::println!("Hello, world!");
    koros_core::println!("cmdline: {:?}", cmdline::raw());

    // Discover and bind device drivers (device-tree `compatible` matching on
    // FDT platforms, PCI enumeration on x86_64).
    probe_devices();

    // Bring up the other CPUs (they register online and idle for now).
    let online = koros_core::smp::boot_secondaries();
    koros_core::println!(
        "SMP: {} CPU(s) online (boot cpu {})",
        online,
        koros_core::smp::cpu_id()
    );

    // `bench` on the kernel command line runs the storage throughput
    // benchmark instead of the functional self-check.
    #[cfg(any(target_arch = "riscv64", target_arch = "x86_64"))]
    if cmdline::has_flag("bench") {
        koros_core::bench::run();
        loop {
            core::hint::spin_loop();
        }
    }

    koros_core::ext2_test();

    loop {
        core::hint::spin_loop();
    }
}

/// Match device-tree nodes against the driver registry and probe them.
///
/// The registry — the set of drivers this kernel image enables — is assembled
/// here in the binary crate; the driver *implementations* and the matching
/// machinery live in `koros_core`.
#[cfg(any(
    target_arch = "riscv64",
    target_arch = "aarch64",
    target_arch = "loongarch64"
))]
fn probe_devices() {
    use koros_core::drivers::driver::{probe_fdt, DeviceDriver};
    use koros_core::drivers::virtio::VIRTIO_MMIO_DRIVER;

    static DRIVERS: &[&dyn DeviceDriver] = &[&VIRTIO_MMIO_DRIVER];
    probe_fdt(mm::dtb_ptr(), DRIVERS);

    // Some FDT platforms (QEMU loongarch `virt`) carry virtio on PCIe instead.
    if let Some(pci) = platform::pci_ecam() {
        koros_core::drivers::virtio::probe_pci_ecam_and_register(
            pci.ecam_base,
            pci.mmio_base,
            pci.mmio_size,
        );
    }
}

#[cfg(target_arch = "x86_64")]
fn probe_devices() {
    // x86_64 has no device tree; enumerate PCI instead.
    koros_core::drivers::virtio::probe_pci_and_register();
}
