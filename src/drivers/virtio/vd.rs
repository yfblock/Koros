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
// MMIO discovery (riscv64 / aarch64)
// ---------------------------------------------------------------------------

/// Probe the virtio-mmio device slots for a block device.
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
pub fn discover_mmio_blk()
-> Option<VdBlk<virtio_drivers::transport::mmio::MmioTransport<'static>>> {
    use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
    use virtio_drivers::transport::DeviceType;

    const MMIO_BASE: usize = if cfg!(target_arch = "riscv64") {
        0x1000_1000
    } else {
        0x0a00_0000
    };
    const MMIO_STRIDE: usize = if cfg!(target_arch = "riscv64") {
        0x1000
    } else {
        0x200
    };
    const MMIO_SIZE: usize = 0x1000;

    crate::println!("virtio-drivers: probing virtio-mmio devices...");
    for i in 0..32usize {
        let base = MMIO_BASE + i * MMIO_STRIDE;
        let header = NonNull::new(base as *mut VirtIOHeader)?;
        // SAFETY: `base` addresses an identity-mapped MMIO register region
        // that remains valid for the whole kernel lifetime ('static).
        let transport = match unsafe { MmioTransport::new(header, MMIO_SIZE) } {
            Ok(t) => t,
            Err(_) => continue,
        };
        if transport.device_type() != DeviceType::Block {
            continue;
        }
        match VirtIOBlk::<KorosHal, _>::new(transport) {
            Ok(blk) => {
                crate::println!(
                    "virtio-drivers: found virtio-blk (mmio) at {:#x} ({} sectors)",
                    base,
                    blk.capacity()
                );
                return Some(VdBlk::from_blk(blk));
            }
            Err(e) => crate::println!("virtio-drivers: VirtIOBlk init failed: {:?}", e),
        }
    }
    None
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
