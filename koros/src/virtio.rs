//! Composition-owned VirtIO registration and interrupt policy.

extern crate alloc;

#[cfg(any(
    target_arch = "riscv64",
    target_arch = "aarch64",
    target_arch = "loongarch64"
))]
use alloc::sync::Arc;
#[cfg(any(
    target_arch = "riscv64",
    target_arch = "aarch64",
    target_arch = "loongarch64"
))]
use kor::driver::DtDevice;
#[cfg(any(
    target_arch = "riscv64",
    target_arch = "aarch64",
    target_arch = "loongarch64"
))]
use kor_virtio::InterruptBlockDevice;

#[cfg(any(
    target_arch = "riscv64",
    target_arch = "aarch64",
    target_arch = "loongarch64"
))]
fn register_mmio_block(device: &DtDevice, block: Arc<dyn InterruptBlockDevice>) {
    if let Some(irq) = kor::enable_device_irq(&device.interrupts) {
        let handler = Arc::clone(&block);
        crate::registries::IRQS.register(
            irq,
            alloc::boxed::Box::new(move || handler.handle_interrupt()),
        );
        block.enable_interrupts();
    }
    crate::registries::BLOCKS.register(block);
}

/// VirtIO MMIO driver configured with Koros' registry and IRQ policy.
#[cfg(any(
    target_arch = "riscv64",
    target_arch = "aarch64",
    target_arch = "loongarch64"
))]
pub static VIRTIO_MMIO_DRIVER: kor_virtio::VirtioMmioDriver =
    kor_virtio::VirtioMmioDriver::new(register_mmio_block);

/// Probe PCI ECAM and register the first VirtIO block device found.
#[cfg(any(
    target_arch = "riscv64",
    target_arch = "aarch64",
    target_arch = "loongarch64"
))]
pub fn probe_pci_ecam_and_register(ecam_base: usize, mmio_base: u64, mmio_size: u64) {
    if let Some(block) = kor_virtio::discover_pci_ecam_blk(ecam_base, mmio_base, mmio_size) {
        crate::registries::BLOCKS.register(block);
    }
}

/// Probe x86 PCI and register the first VirtIO block device found.
#[cfg(target_arch = "x86_64")]
pub fn probe_pci_and_register() {
    if let Some(block) = kor_virtio::discover_pci_blk() {
        crate::registries::BLOCKS.register(block);
    }
}
