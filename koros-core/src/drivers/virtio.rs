//! Adapter for the external [`virtio_drivers`] crate.
//!
//! Implements the crate's [`Hal`] against Koros' physical-frame allocator and
//! direct map, and wraps its `VirtIOBlk` (over either the MMIO or PCI
//! transport) in the kernel's own [`BlockDevice`] trait so it can back the
//! ext2 filesystem.

extern crate alloc;

use core::ptr::NonNull;
use spin::Mutex;

use virtio_drivers::device::blk::{VirtIOBlk, SECTOR_SIZE};
use virtio_drivers::transport::Transport;
use virtio_drivers::{BufferDirection, Hal, PhysAddr};

use crate::drivers::block::{BlockDevice, BlockError};
use crate::mm;

// ---------------------------------------------------------------------------
// Hal — bridges virtio-drivers DMA/address needs to Koros' mm subsystem
// ---------------------------------------------------------------------------

/// Koros implementation of the virtio-drivers hardware-abstraction layer.
pub struct KorosHal;

// SAFETY: DMA buffers come from the physical frame allocator and are accessed
// through the kernel direct map; physical/virtual translation uses the mm
// direct-map helpers, satisfying the `Hal` contract.
unsafe impl Hal for KorosHal {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        let pa = mm::alloc_frames(pages).expect("virtio dma_alloc: out of frames");
        let va = mm::phys_to_virt(pa);
        // The frame allocator does not zero; `Hal` requires zeroed pages.
        unsafe {
            core::ptr::write_bytes(va as *mut u8, 0, pages * mm::PAGE_SIZE);
        }
        (pa as PhysAddr, NonNull::new(va as *mut u8).unwrap())
    }

    unsafe fn dma_dealloc(paddr: PhysAddr, _vaddr: NonNull<u8>, pages: usize) -> i32 {
        mm::free_frames(paddr as usize, pages);
        0
    }

    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, _size: usize) -> NonNull<u8> {
        NonNull::new(mm::phys_to_virt(paddr as usize) as *mut u8).unwrap()
    }

    unsafe fn share(buffer: NonNull<[u8]>, _direction: BufferDirection) -> PhysAddr {
        // Driver buffers live in the kernel direct map, so a direct-map
        // translation yields the physical address the device should use.
        let va = buffer.as_ptr() as *mut u8 as usize;
        mm::virt_to_phys(va) as PhysAddr
    }

    unsafe fn unshare(_paddr: PhysAddr, _buffer: NonNull<[u8]>, _direction: BufferDirection) {}
}

// ---------------------------------------------------------------------------
// VdBlk — BlockDevice backed by virtio-drivers' VirtIOBlk over any transport
// ---------------------------------------------------------------------------

/// A block device driven by the `virtio_drivers` crate.
pub struct VdBlk<T: Transport> {
    inner: Mutex<VirtIOBlk<KorosHal, T>>,
    capacity_sectors: u64,
}

// SAFETY: all access to the inner driver is serialised by the mutex.
unsafe impl<T: Transport> Send for VdBlk<T> {}
unsafe impl<T: Transport> Sync for VdBlk<T> {}

impl<T: Transport + 'static> BlockDevice for VdBlk<T> {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) -> Result<(), BlockError> {
        self.inner
            .lock()
            .read_blocks(block_id, buf)
            .map_err(|_| BlockError::IoError)
    }

    fn write_block(&self, block_id: usize, buf: &[u8]) -> Result<(), BlockError> {
        self.inner
            .lock()
            .write_blocks(block_id, buf)
            .map_err(|_| BlockError::IoError)
    }

    fn block_size(&self) -> usize {
        SECTOR_SIZE
    }

    fn total_blocks(&self) -> usize {
        self.capacity_sectors as usize
    }

    fn read_blocks(&self, start_block: usize, buf: &mut [u8]) -> Result<(), BlockError> {
        // virtio-drivers accepts a buffer spanning multiple sectors and issues
        // it as a single multi-descriptor request.
        self.inner
            .lock()
            .read_blocks(start_block, buf)
            .map_err(|_| BlockError::IoError)
    }

    fn write_blocks(&self, start_block: usize, buf: &[u8]) -> Result<(), BlockError> {
        self.inner
            .lock()
            .write_blocks(start_block, buf)
            .map_err(|_| BlockError::IoError)
    }
}

impl<T: Transport> VdBlk<T> {
    fn from_blk(blk: VirtIOBlk<KorosHal, T>) -> Self {
        let capacity_sectors = blk.capacity();
        Self {
            inner: Mutex::new(blk),
            capacity_sectors,
        }
    }
}

// ---------------------------------------------------------------------------
// virtio-mmio driver (matched by the device-tree `compatible = "virtio,mmio"`)
// ---------------------------------------------------------------------------

#[cfg(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "loongarch64"))]
use crate::drivers::driver::{DeviceDriver, DtDevice, DriverError};

/// The virtio-mmio bus/transport driver.  Its `probe` builds the transport at
/// the node's `reg` base, reads the device type, and — for block devices —
/// registers a [`VdBlk`] in the global block-device registry.
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "loongarch64"))]
pub struct VirtioMmioDriver;

/// Singleton instance, referenced by the binary crate's driver registry.
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "loongarch64"))]
pub static VIRTIO_MMIO_DRIVER: VirtioMmioDriver = VirtioMmioDriver;

#[cfg(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "loongarch64"))]
impl DeviceDriver for VirtioMmioDriver {
    fn compatible(&self) -> &'static [&'static str] {
        &["virtio,mmio"]
    }

    fn probe(&self, dev: &DtDevice) -> Result<(), DriverError> {
        use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
        use virtio_drivers::transport::{DeviceType, Transport};

        let header =
            NonNull::new(dev.reg_base as *mut VirtIOHeader).ok_or(DriverError::NoResource)?;
        // SAFETY: `reg_base`/`reg_size` describe a valid MMIO register region
        // from the device tree, valid for the whole kernel lifetime.
        let transport = match unsafe { MmioTransport::new(header, dev.reg_size) } {
            Ok(t) => t,
            // An empty virtio-mmio slot (device id 0) is normal, not an error.
            Err(_) => return Ok(()),
        };
        match transport.device_type() {
            DeviceType::Block => {
                let blk =
                    VirtIOBlk::<KorosHal, _>::new(transport).map_err(|_| DriverError::Probe)?;
                crate::println!(
                    "virtio-blk (mmio) at {:#x}: {} sectors",
                    dev.reg_base,
                    blk.capacity()
                );
                crate::drivers::block::register(alloc::sync::Arc::new(VdBlk::from_blk(blk)));
                Ok(())
            }
            // Other virtio device types are recognised but not yet supported.
            _ => Ok(()),
        }
    }
}

// ---------------------------------------------------------------------------
// PCIe discovery via memory-mapped ECAM (loongarch64 / other FDT platforms
// whose virtio devices sit on PCIe instead of a virtio-mmio bus)
// ---------------------------------------------------------------------------

/// Assign MMIO addresses to every unallocated memory BAR of `df` from a bump
/// allocator starting at `*next`.  QEMU's loongarch `virt` machine has no
/// firmware to program BARs, so the kernel must do it before using the device.
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "loongarch64"))]
fn allocate_bars<C>(
    root: &mut virtio_drivers::transport::pci::bus::PciRoot<C>,
    df: virtio_drivers::transport::pci::bus::DeviceFunction,
    next: &mut u64,
) where
    C: virtio_drivers::transport::pci::bus::ConfigurationAccess,
{
    use virtio_drivers::transport::pci::bus::{BarInfo, MemoryBarType};

    let mut bar = 0u8;
    while bar < 6 {
        let info = match root.bar_info(df, bar) {
            Ok(Some(info)) => info,
            _ => {
                bar += 1;
                continue;
            }
        };
        let two = info.takes_two_entries();
        if let BarInfo::Memory { address_type, size, .. } = info {
            if size > 0 {
                let aligned = (*next + (size - 1)) & !(size - 1);
                match address_type {
                    MemoryBarType::Width64 => root.set_bar_64(df, bar, aligned),
                    _ => root.set_bar_32(df, bar, aligned as u32),
                }
                *next = aligned + size;
            }
        }
        bar += if two { 2 } else { 1 };
    }
}

/// Enumerate an ECAM PCIe root complex, program the first virtio-blk device's
/// BARs, and register it as the block device.
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "loongarch64"))]
pub fn probe_pci_ecam_and_register(ecam_base: usize, mmio_base: u64, mmio_size: u64) {
    use virtio_drivers::transport::pci::bus::{Cam, Command, DeviceFunction, MmioCam, PciRoot};
    use virtio_drivers::transport::pci::{virtio_device_type, PciTransport};
    use virtio_drivers::transport::DeviceType;

    let ecam_va = mm::phys_to_virt(ecam_base);
    crate::println!("virtio-drivers: scanning PCIe ECAM at {:#x} for virtio-blk...", ecam_base);
    // SAFETY: `ecam_va` maps the platform's ECAM configuration region.
    let cam = unsafe { MmioCam::new(ecam_va as *mut u8, Cam::Ecam) };
    let mut root = PciRoot::new(cam);

    let mut target: Option<DeviceFunction> = None;
    for (df, info) in root.enumerate_bus(0) {
        if matches!(virtio_device_type(&info), Some(DeviceType::Block)) {
            target = Some(df);
            break;
        }
    }
    let Some(df) = target else {
        return;
    };

    let mut next = mmio_base;
    allocate_bars(&mut root, df, &mut next);
    if next > mmio_base + mmio_size {
        crate::println!("virtio-drivers: PCIe MMIO window exhausted");
        return;
    }

    // Enable memory-space decoding and bus mastering (for DMA).
    let (_status, command) = root.get_status_command(df);
    root.set_command(df, command | Command::MEMORY_SPACE | Command::BUS_MASTER);

    let transport = match PciTransport::new::<KorosHal, _>(&mut root, df) {
        Ok(t) => t,
        Err(e) => {
            crate::println!("virtio-drivers: PciTransport::new failed: {:?}", e);
            return;
        }
    };
    match VirtIOBlk::<KorosHal, _>::new(transport) {
        Ok(blk) => {
            crate::println!(
                "virtio-drivers: found virtio-blk (pcie) {:?} ({} sectors)",
                df,
                blk.capacity()
            );
            crate::drivers::block::register(alloc::sync::Arc::new(VdBlk::from_blk(blk)));
        }
        Err(e) => crate::println!("virtio-drivers: VirtIOBlk init failed: {:?}", e),
    }
}

// ---------------------------------------------------------------------------
// PCI discovery (x86_64) — config access via legacy port I/O (0xCF8/0xCFC)
// ---------------------------------------------------------------------------

/// PCI configuration access using x86 legacy I/O ports.
#[cfg(target_arch = "x86_64")]
struct PortCam;

#[cfg(target_arch = "x86_64")]
impl virtio_drivers::transport::pci::bus::ConfigurationAccess for PortCam {
    fn read_word(
        &self,
        device_function: virtio_drivers::transport::pci::bus::DeviceFunction,
        register_offset: u8,
    ) -> u32 {
        use x86_64::instructions::port::Port;
        let addr = pci_cfg_address(device_function, register_offset);
        unsafe {
            Port::<u32>::new(0xCF8).write(addr);
            Port::<u32>::new(0xCFC).read()
        }
    }

    fn write_word(
        &mut self,
        device_function: virtio_drivers::transport::pci::bus::DeviceFunction,
        register_offset: u8,
        data: u32,
    ) {
        use x86_64::instructions::port::Port;
        let addr = pci_cfg_address(device_function, register_offset);
        unsafe {
            Port::<u32>::new(0xCF8).write(addr);
            Port::<u32>::new(0xCFC).write(data);
        }
    }

    unsafe fn unsafe_clone(&self) -> Self {
        PortCam
    }
}

/// Build the 0xCF8 configuration address for a `(device_function, offset)`.
#[cfg(target_arch = "x86_64")]
fn pci_cfg_address(
    df: virtio_drivers::transport::pci::bus::DeviceFunction,
    register_offset: u8,
) -> u32 {
    0x8000_0000
        | ((df.bus as u32) << 16)
        | ((df.device as u32) << 11)
        | ((df.function as u32) << 8)
        | (register_offset as u32 & 0xFC)
}

/// Probe the PCI bus for a virtio block device.
#[cfg(target_arch = "x86_64")]
pub fn discover_pci_blk() -> Option<VdBlk<virtio_drivers::transport::pci::PciTransport>> {
    use virtio_drivers::transport::pci::bus::{Command, DeviceFunction, PciRoot};
    use virtio_drivers::transport::pci::{virtio_device_type, PciTransport};
    use virtio_drivers::transport::DeviceType;

    crate::println!("virtio-drivers: scanning PCI bus for virtio-blk...");
    let mut root = PciRoot::new(PortCam);

    // Find the first virtio block device on bus 0 (collect first so the
    // immutable enumeration borrow is released before mutating below).
    let mut target: Option<DeviceFunction> = None;
    for (df, info) in root.enumerate_bus(0) {
        if matches!(virtio_device_type(&info), Some(DeviceType::Block)) {
            target = Some(df);
            break;
        }
    }
    let df = target?;

    // Enable memory-space decoding and bus mastering (for DMA).
    let (_status, command) = root.get_status_command(df);
    root.set_command(df, command | Command::MEMORY_SPACE | Command::BUS_MASTER);

    let transport = match PciTransport::new::<KorosHal, _>(&mut root, df) {
        Ok(t) => t,
        Err(e) => {
            crate::println!("virtio-drivers: PciTransport::new failed: {:?}", e);
            return None;
        }
    };
    match VirtIOBlk::<KorosHal, _>::new(transport) {
        Ok(blk) => {
            crate::println!(
                "virtio-drivers: found virtio-blk (pci) {:?} ({} sectors)",
                df,
                blk.capacity()
            );
            Some(VdBlk::from_blk(blk))
        }
        Err(e) => {
            crate::println!("virtio-drivers: VirtIOBlk init failed: {:?}", e);
            None
        }
    }
}

/// Scan the PCI bus for a virtio block device and register it (x86_64 has no
/// device tree, so it uses PCI enumeration instead of `compatible` matching).
#[cfg(target_arch = "x86_64")]
pub fn probe_pci_and_register() {
    if let Some(blk) = discover_pci_blk() {
        crate::drivers::block::register(alloc::sync::Arc::new(blk));
    }
}
