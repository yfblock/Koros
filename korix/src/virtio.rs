//! Composition-owned VirtIO registration policy for Korix.

extern crate alloc;

#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
use alloc::sync::Arc;
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
use kor::driver::DtDevice;
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
use kor_virtio::InterruptBlockDevice;

#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
fn register_mmio_block(_device: &DtDevice, block: Arc<dyn InterruptBlockDevice>) {
    crate::registries::BLOCKS.register(block);
}

/// VirtIO MMIO driver configured with Korix's polling and registry policy.
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
pub static VIRTIO_MMIO_DRIVER: kor_virtio::VirtioMmioDriver =
    kor_virtio::VirtioMmioDriver::new(register_mmio_block);

/// Probe x86 PCI and register the first VirtIO block device found.
#[cfg(target_arch = "x86_64")]
pub fn probe_pci_and_register() {
    if let Some(block) = kor_virtio::discover_pci_blk() {
        crate::registries::BLOCKS.register(block);
    }
}
