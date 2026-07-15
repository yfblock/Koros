#![no_std]

//! Shared VirtIO block HAL, adapter, and transport probing support.
//!
//! Device registries and the policy for wiring interrupts remain in each
//! composition crate. This crate only constructs block devices and reports
//! discovered devices through callbacks or return values.

extern crate alloc;

use alloc::sync::Arc;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, Ordering};
use kor::{BlockDevice, BlockError};
use spin::Mutex;
use virtio_drivers::device::blk::{BlkReq, BlkResp, SECTOR_SIZE, VirtIOBlk};
use virtio_drivers::transport::Transport;
use virtio_drivers::{BufferDirection, Hal, PhysAddr};

/// Koros workspace implementation of the `virtio-drivers` HAL.
pub struct KorHal;

// SAFETY: DMA buffers come from the physical frame allocator and are accessed
// through the kernel direct map. Address translation uses the installed
// architecture provider, satisfying the `Hal` contract.
unsafe impl Hal for KorHal {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        let physical = kor_frame::alloc_frames(pages).expect("virtio dma_alloc: out of frames");
        let virtual_address = kor::arch::phys_to_virt(physical);
        // SAFETY: the allocator returned `pages` contiguous, exclusively owned
        // frames and the architecture direct map covers those frames.
        unsafe {
            core::ptr::write_bytes(virtual_address as *mut u8, 0, pages * kor_frame::PAGE_SIZE);
        }
        (
            physical as PhysAddr,
            NonNull::new(virtual_address as *mut u8).expect("direct map returned null"),
        )
    }

    unsafe fn dma_dealloc(paddr: PhysAddr, _vaddr: NonNull<u8>, pages: usize) -> i32 {
        kor_frame::free_frames(paddr as usize, pages);
        0
    }

    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, _size: usize) -> NonNull<u8> {
        NonNull::new(kor::arch::phys_to_virt(paddr as usize) as *mut u8)
            .expect("direct map returned null")
    }

    unsafe fn share(buffer: NonNull<[u8]>, _direction: BufferDirection) -> PhysAddr {
        let virtual_address = buffer.as_ptr() as *mut u8 as usize;
        kor::arch::virt_to_phys(virtual_address) as PhysAddr
    }

    unsafe fn unshare(_paddr: PhysAddr, _buffer: NonNull<[u8]>, _direction: BufferDirection) {}
}

/// A block device backed by `virtio-drivers` over any transport.
pub struct VirtioBlock<T: Transport> {
    inner: Mutex<VirtIOBlk<KorHal, T>>,
    capacity_sectors: u64,
    op_lock: kor_sched::sync::Mutex<()>,
    completion: kor_sched::Semaphore,
    irq_driven: AtomicBool,
}

// SAFETY: every access to the transport driver is serialized by `inner`.
unsafe impl<T: Transport> Send for VirtioBlock<T> {}
// SAFETY: every access to the transport driver is serialized by `inner`.
unsafe impl<T: Transport> Sync for VirtioBlock<T> {}

impl<T: Transport + 'static> BlockDevice for VirtioBlock<T> {
    fn read_block(&self, block_id: usize, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.read_blocks(block_id, buffer)
    }

    fn write_block(&self, block_id: usize, buffer: &[u8]) -> Result<(), BlockError> {
        self.write_blocks(block_id, buffer)
    }

    fn flush(&self) -> Result<(), BlockError> {
        if self.use_irq() {
            let _operation = self.op_lock.lock();
            return kor::irq::without(|| {
                self.inner.lock().flush().map_err(|_| BlockError::IoError)
            });
        }
        kor::irq::without(|| self.inner.lock().flush().map_err(|_| BlockError::IoError))
    }

    fn block_size(&self) -> usize {
        SECTOR_SIZE
    }

    fn total_blocks(&self) -> usize {
        self.capacity_sectors as usize
    }

    fn read_blocks(&self, start_block: usize, buffer: &mut [u8]) -> Result<(), BlockError> {
        if self.use_irq() {
            return self.read_irq(start_block, buffer);
        }
        kor::irq::without(|| {
            self.inner
                .lock()
                .read_blocks(start_block, buffer)
                .map_err(|_| BlockError::IoError)
        })
    }

    fn write_blocks(&self, start_block: usize, buffer: &[u8]) -> Result<(), BlockError> {
        if self.use_irq() {
            return self.write_irq(start_block, buffer);
        }
        kor::irq::without(|| {
            self.inner
                .lock()
                .write_blocks(start_block, buffer)
                .map_err(|_| BlockError::IoError)
        })
    }
}

impl<T: Transport> VirtioBlock<T> {
    fn new(block: VirtIOBlk<KorHal, T>) -> Self {
        let capacity_sectors = block.capacity();
        Self {
            inner: Mutex::new(block),
            capacity_sectors,
            op_lock: kor_sched::sync::Mutex::new(()),
            completion: kor_sched::Semaphore::new(0),
            irq_driven: AtomicBool::new(false),
        }
    }

    fn use_irq(&self) -> bool {
        self.irq_driven.load(Ordering::Relaxed) && kor_sched::is_ready()
    }

    fn read_irq(&self, block_id: usize, buffer: &mut [u8]) -> Result<(), BlockError> {
        let _operation = self.op_lock.lock();
        let mut request = BlkReq::default();
        let mut response = BlkResp::default();
        let token = kor::irq::without(|| {
            // SAFETY: request, buffer, and response remain live and untouched
            // until the matching completion call below.
            unsafe {
                self.inner
                    .lock()
                    .read_blocks_nb(block_id, &mut request, buffer, &mut response)
            }
        })
        .map_err(|_| BlockError::IoError)?;
        loop {
            self.completion.wait();
            let completed = kor::irq::without(|| {
                let mut block = self.inner.lock();
                if block.peek_used() == Some(token) {
                    // SAFETY: these are the buffers supplied to the matching
                    // non-blocking request, and that token has completed.
                    Some(unsafe {
                        block.complete_read_blocks(token, &request, buffer, &mut response)
                    })
                } else {
                    None
                }
            });
            if let Some(result) = completed {
                return result.map_err(|_| BlockError::IoError);
            }
        }
    }

    fn write_irq(&self, block_id: usize, buffer: &[u8]) -> Result<(), BlockError> {
        let _operation = self.op_lock.lock();
        let mut request = BlkReq::default();
        let mut response = BlkResp::default();
        let token = kor::irq::without(|| {
            // SAFETY: request, buffer, and response remain live and untouched
            // until the matching completion call below.
            unsafe {
                self.inner
                    .lock()
                    .write_blocks_nb(block_id, &mut request, buffer, &mut response)
            }
        })
        .map_err(|_| BlockError::IoError)?;
        loop {
            self.completion.wait();
            let completed = kor::irq::without(|| {
                let mut block = self.inner.lock();
                if block.peek_used() == Some(token) {
                    // SAFETY: these are the buffers supplied to the matching
                    // non-blocking request, and that token has completed.
                    Some(unsafe {
                        block.complete_write_blocks(token, &request, buffer, &mut response)
                    })
                } else {
                    None
                }
            });
            if let Some(result) = completed {
                return result.map_err(|_| BlockError::IoError);
            }
        }
    }
}

/// Interrupt controls exposed to a composition-owned IRQ registry.
pub trait InterruptBlockDevice: BlockDevice {
    /// Acknowledge a device interrupt and wake a pending request.
    fn handle_interrupt(&self);

    /// Enable interrupt-driven requests after an IRQ handler has been installed.
    fn enable_interrupts(&self);
}

impl<T: Transport + 'static> InterruptBlockDevice for VirtioBlock<T> {
    fn handle_interrupt(&self) {
        kor::irq::without(|| {
            self.inner.lock().ack_interrupt();
        });
        self.completion.post();
    }

    fn enable_interrupts(&self) {
        self.irq_driven.store(true, Ordering::Relaxed);
    }
}

/// Callback used by MMIO probing to hand a discovered device to a composition root.
pub type MmioBlockFound = fn(&kor::driver::DtDevice, Arc<dyn InterruptBlockDevice>);

/// Device-tree driver for VirtIO MMIO block devices.
pub struct VirtioMmioDriver {
    block_found: MmioBlockFound,
}

impl VirtioMmioDriver {
    /// Construct a driver that reports each block device through `block_found`.
    pub const fn new(block_found: MmioBlockFound) -> Self {
        Self { block_found }
    }
}

impl kor::driver::DeviceDriver for VirtioMmioDriver {
    fn compatible(&self) -> &'static [&'static str] {
        &["virtio,mmio"]
    }

    fn probe(&self, device: &kor::driver::DtDevice) -> Result<(), kor::driver::DriverError> {
        use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
        use virtio_drivers::transport::{DeviceType, Transport};

        let header = NonNull::new(kor::arch::phys_to_virt(device.reg_base) as *mut VirtIOHeader)
            .ok_or(kor::driver::DriverError::NoResource)?;
        // SAFETY: the device-tree register range is mapped for the kernel
        // lifetime and points to a VirtIO MMIO register block.
        let transport = match unsafe { MmioTransport::new(header, device.reg_size) } {
            Ok(transport) => transport,
            Err(_) => return Ok(()),
        };
        if transport.device_type() != DeviceType::Block {
            return Ok(());
        }

        let block =
            VirtIOBlk::<KorHal, _>::new(transport).map_err(|_| kor::driver::DriverError::Probe)?;
        kor::println!(
            "virtio-blk (mmio) at {:#x}: {} sectors",
            device.reg_base,
            block.capacity()
        );
        (self.block_found)(device, Arc::new(VirtioBlock::new(block)));
        Ok(())
    }
}

/// Assign MMIO addresses to every unallocated memory BAR of a PCI function.
fn allocate_bars<C>(
    root: &mut virtio_drivers::transport::pci::bus::PciRoot<C>,
    device_function: virtio_drivers::transport::pci::bus::DeviceFunction,
    next: &mut u64,
) where
    C: virtio_drivers::transport::pci::bus::ConfigurationAccess,
{
    use virtio_drivers::transport::pci::bus::{BarInfo, MemoryBarType};

    const PCI_BAR_COUNT: u8 = 6;
    let mut bar = 0;
    while bar < PCI_BAR_COUNT {
        let info = match root.bar_info(device_function, bar) {
            Ok(Some(info)) => info,
            _ => {
                bar += 1;
                continue;
            }
        };
        let takes_two_entries = info.takes_two_entries();
        if let BarInfo::Memory {
            address_type, size, ..
        } = info
        {
            if size > 0 {
                let aligned = (*next + (size - 1)) & !(size - 1);
                match address_type {
                    MemoryBarType::Width64 => root.set_bar_64(device_function, bar, aligned),
                    _ => root.set_bar_32(device_function, bar, aligned as u32),
                }
                *next = aligned + size;
            }
        }
        bar += if takes_two_entries { 2 } else { 1 };
    }
}

/// Probe an ECAM root for the first VirtIO block device.
pub fn discover_pci_ecam_blk(
    ecam_base: usize,
    mmio_base: u64,
    mmio_size: u64,
) -> Option<Arc<dyn BlockDevice>> {
    use virtio_drivers::transport::DeviceType;
    use virtio_drivers::transport::pci::bus::{Cam, Command, DeviceFunction, MmioCam, PciRoot};
    use virtio_drivers::transport::pci::{PciTransport, virtio_device_type};

    const ROOT_BUS: u8 = 0;
    let ecam_virtual = kor::arch::phys_to_virt(ecam_base);
    kor::println!(
        "virtio-drivers: scanning PCIe ECAM at {:#x} for virtio-blk...",
        ecam_base
    );
    // SAFETY: `ecam_virtual` maps the configured platform ECAM region.
    let cam = unsafe { MmioCam::new(ecam_virtual as *mut u8, Cam::Ecam) };
    let mut root = PciRoot::new(cam);
    let mut target: Option<DeviceFunction> = None;
    for (device_function, info) in root.enumerate_bus(ROOT_BUS) {
        if matches!(virtio_device_type(&info), Some(DeviceType::Block)) {
            target = Some(device_function);
            break;
        }
    }
    let device_function = target?;

    let mut next = mmio_base;
    allocate_bars(&mut root, device_function, &mut next);
    if next > mmio_base + mmio_size {
        kor::println!("virtio-drivers: PCIe MMIO window exhausted");
        return None;
    }
    let (_status, command) = root.get_status_command(device_function);
    root.set_command(
        device_function,
        command | Command::MEMORY_SPACE | Command::BUS_MASTER,
    );
    let transport = match PciTransport::new::<KorHal, _>(&mut root, device_function) {
        Ok(transport) => transport,
        Err(error) => {
            kor::println!("virtio-drivers: PciTransport::new failed: {:?}", error);
            return None;
        }
    };
    match VirtIOBlk::<KorHal, _>::new(transport) {
        Ok(block) => {
            kor::println!(
                "virtio-drivers: found virtio-blk (pcie) {:?} ({} sectors)",
                device_function,
                block.capacity()
            );
            Some(Arc::new(VirtioBlock::new(block)))
        }
        Err(error) => {
            kor::println!("virtio-drivers: VirtIOBlk init failed: {:?}", error);
            None
        }
    }
}

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
        const PCI_CONFIG_ADDRESS_PORT: u16 = 0xCF8;
        const PCI_CONFIG_DATA_PORT: u16 = 0xCFC;
        let address = pci_cfg_address(device_function, register_offset);
        // SAFETY: PCI configuration mechanism 1 reserves these ports for
        // aligned configuration-space transactions.
        unsafe {
            Port::<u32>::new(PCI_CONFIG_ADDRESS_PORT).write(address);
            Port::<u32>::new(PCI_CONFIG_DATA_PORT).read()
        }
    }

    fn write_word(
        &mut self,
        device_function: virtio_drivers::transport::pci::bus::DeviceFunction,
        register_offset: u8,
        data: u32,
    ) {
        use x86_64::instructions::port::Port;
        const PCI_CONFIG_ADDRESS_PORT: u16 = 0xCF8;
        const PCI_CONFIG_DATA_PORT: u16 = 0xCFC;
        let address = pci_cfg_address(device_function, register_offset);
        // SAFETY: PCI configuration mechanism 1 reserves these ports for
        // aligned configuration-space transactions.
        unsafe {
            Port::<u32>::new(PCI_CONFIG_ADDRESS_PORT).write(address);
            Port::<u32>::new(PCI_CONFIG_DATA_PORT).write(data);
        }
    }

    unsafe fn unsafe_clone(&self) -> Self {
        Self
    }
}

#[cfg(target_arch = "x86_64")]
fn pci_cfg_address(
    device_function: virtio_drivers::transport::pci::bus::DeviceFunction,
    register_offset: u8,
) -> u32 {
    const PCI_CONFIG_ENABLE: u32 = 0x8000_0000;
    const PCI_REGISTER_MASK: u32 = 0xFC;
    PCI_CONFIG_ENABLE
        | ((device_function.bus as u32) << 16)
        | ((device_function.device as u32) << 11)
        | ((device_function.function as u32) << 8)
        | (register_offset as u32 & PCI_REGISTER_MASK)
}

/// Probe x86 PCI configuration mechanism 1 for the first VirtIO block device.
#[cfg(target_arch = "x86_64")]
pub fn discover_pci_blk() -> Option<Arc<dyn BlockDevice>> {
    use virtio_drivers::transport::DeviceType;
    use virtio_drivers::transport::pci::bus::{Command, DeviceFunction, PciRoot};
    use virtio_drivers::transport::pci::{PciTransport, virtio_device_type};

    const ROOT_BUS: u8 = 0;
    kor::println!("virtio-drivers: scanning PCI bus for virtio-blk...");
    let mut root = PciRoot::new(PortCam);
    let mut target: Option<DeviceFunction> = None;
    for (device_function, info) in root.enumerate_bus(ROOT_BUS) {
        if matches!(virtio_device_type(&info), Some(DeviceType::Block)) {
            target = Some(device_function);
            break;
        }
    }
    let device_function = target?;
    let (_status, command) = root.get_status_command(device_function);
    root.set_command(
        device_function,
        command | Command::MEMORY_SPACE | Command::BUS_MASTER,
    );
    let transport = match PciTransport::new::<KorHal, _>(&mut root, device_function) {
        Ok(transport) => transport,
        Err(error) => {
            kor::println!("virtio-drivers: PciTransport::new failed: {:?}", error);
            return None;
        }
    };
    match VirtIOBlk::<KorHal, _>::new(transport) {
        Ok(block) => {
            kor::println!(
                "virtio-drivers: found virtio-blk (pci) {:?} ({} sectors)",
                device_function,
                block.capacity()
            );
            Some(Arc::new(VirtioBlock::new(block)))
        }
        Err(error) => {
            kor::println!("virtio-drivers: VirtIOBlk init failed: {:?}", error);
            None
        }
    }
}
