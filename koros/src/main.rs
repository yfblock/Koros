#![no_std]
#![no_main]

use koros_core::{cmdline, mm, trap};

/// Kernel entry point, called by the architecture boot code
/// (`koros_core::arch::<arch>::boot::rust_entry`) after early setup.
#[unsafe(no_mangle)]
extern "C" fn kernel_main() -> ! {
    trap::init();
    mm::init(); // captures the boot command line early (see mm::init)
    koros_core::println!("Hello, world!");
    koros_core::println!("cmdline: {:?}", cmdline::raw());

    // Discover and bind device drivers (device-tree `compatible` matching on
    // FDT platforms, PCI enumeration on x86_64).
    probe_devices();

    // `bench` on the kernel command line runs the storage throughput
    // benchmark instead of the functional self-check.
    #[cfg(any(target_arch = "riscv64", target_arch = "x86_64"))]
    if cmdline::has_flag("bench") {
        koros_core::bench::run();
        loop {
            core::hint::spin_loop();
        }
    }

    #[cfg(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "x86_64"))]
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
}

#[cfg(target_arch = "x86_64")]
fn probe_devices() {
    // x86_64 has no device tree; enumerate PCI instead.
    koros_core::drivers::virtio::probe_pci_and_register();
}
