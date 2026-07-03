//! Platform / board configuration.
//!
//! Hardware-specific addresses (console UART, firmware-reserved region, a
//! fixed device-tree address, …) are **not** hardcoded in `koros-core`.
//! Instead the `koros` binary crate — which knows the concrete target board —
//! builds a [`PlatformConfig`] and installs it via [`init`] at the very start
//! of `kernel_main`.  The rest of `koros-core` reads the board addresses back
//! through the accessors here.
//!
//! Note: the kernel virtual base (`KERNEL_OFFSET`) is the one address that
//! stays a compile-time constant, because the boot assembly uses it to build
//! the initial page tables before any Rust runs, and `phys_to_virt` is a hot
//! path.  It is defined by `koros/linker.lds` and mirrored in the per-arch
//! boot code.

use spin::Once;

/// The console UART device: kind + base address.
#[derive(Clone, Copy)]
pub enum Console {
    /// NS16550A over MMIO at `base`.
    Ns16550aMmio { base: usize },
    /// NS16550A over x86 legacy port I/O at `base`.
    Ns16550aPort { base: u16 },
    /// ARM PL011 over MMIO at `base`.
    Pl011Mmio { base: usize },
}

/// A memory-mapped (ECAM) PCIe host bridge, for platforms whose virtio devices
/// live on PCIe rather than on a virtio-mmio bus (e.g. QEMU loongarch `virt`).
#[derive(Clone, Copy)]
pub struct PciEcam {
    /// Physical base of the ECAM configuration space.
    pub ecam_base: usize,
    /// Physical base of the 32-bit MMIO window used to place device BARs.
    pub mmio_base: u64,
    /// Size of that MMIO window in bytes.
    pub mmio_size: u64,
}

/// Board configuration supplied by the binary crate.
#[derive(Clone, Copy)]
pub struct PlatformConfig {
    /// Console UART.
    pub console: Console,
    /// Lowest physical address reserved for firmware; kept out of the frame
    /// allocator (0 if nothing below the kernel image is reserved).
    pub firmware_phys_start: usize,
    /// Fixed device-tree physical address, or 0 to use the pointer the
    /// firmware passed in a register at boot.
    pub dtb: usize,
    /// ECAM PCIe host bridge, if virtio devices are on PCIe.
    pub pci: Option<PciEcam>,
}

static CONFIG: Once<PlatformConfig> = Once::new();

/// Install the platform configuration.  Call once, first thing in
/// `kernel_main`, before any console output or memory setup.
pub fn init(config: PlatformConfig) {
    CONFIG.call_once(|| config);
}

/// The console UART, if the platform configuration has been installed.
pub fn console() -> Option<Console> {
    CONFIG.get().map(|c| c.console)
}

/// Lowest firmware-reserved physical address (0 if none / not yet configured).
pub fn firmware_phys_start() -> usize {
    CONFIG.get().map_or(0, |c| c.firmware_phys_start)
}

/// Fixed device-tree address from the platform config (0 if none).
pub fn config_dtb() -> usize {
    CONFIG.get().map_or(0, |c| c.dtb)
}

/// The ECAM PCIe host bridge, if the platform has one.
pub fn pci_ecam() -> Option<PciEcam> {
    CONFIG.get().and_then(|c| c.pci)
}
