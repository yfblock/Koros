//! Virtio PCI transport implementation.
//!
//! Implements the [`VirtioTransport`] trait for PCI-attached virtio devices.
//! Discovery uses x86_64 port I/O to read PCI config space and enumerate
//! virtio capabilities.  The actual virtio config structures are accessed
//! through memory-mapped BAR regions (same volatile read/write pattern as
//! MMIO transport).
//!
//! References:
//! - virtio spec Section 4.1 (PCI Transport)
//! - PCI Local Bus Specification 3.0, Section 6 (Configuration Space)

use super::VirtioTransport;

// ---------------------------------------------------------------------------
// PCI I/O ports (x86_64)
// ---------------------------------------------------------------------------

/// PCI Configuration Address register (I/O port).
const PCI_CONFIG_ADDR: u16 = 0xCF8;
/// PCI Configuration Data register (I/O port).
const PCI_CONFIG_DATA: u16 = 0xCFC;

// ---------------------------------------------------------------------------
// PCI config-space register offsets
// ---------------------------------------------------------------------------

const PCI_VENDOR_ID: u16 = 0x00;
const PCI_DEVICE_ID: u16 = 0x02;
const PCI_COMMAND: u16 = 0x04;
const PCI_STATUS: u16 = 0x06;
const PCI_CAPABILITIES_PTR: u16 = 0x34;
const PCI_BAR0: u16 = 0x10;

// ---------------------------------------------------------------------------
// Virtio PCI constants
// ---------------------------------------------------------------------------

/// Virtio vendor ID (transitional and modern devices).
pub const VIRTIO_VENDOR_ID: u16 = 0x1AF4;
/// First device ID in the virtio range (transitional block device).
pub const VIRTIO_DEVICE_ID_BASE: u16 = 0x1000;

/// PCI capability type: Virtio PCI Common Configuration.
pub const VIRTIO_PCI_CAP_COMMON_CFG: u8 = 1;
/// PCI capability type: Virtio PCI Notification Configuration.
pub const VIRTIO_PCI_CAP_NOTIFY_CFG: u8 = 2;
/// PCI capability type: Virtio PCI ISR Status.
pub const VIRTIO_PCI_CAP_ISR_CFG: u8 = 3;
/// PCI capability type: Virtio PCI Device-specific Configuration.
pub const VIRTIO_PCI_CAP_DEVICE_CFG: u8 = 4;
/// PCI capability type: Virtio PCI PCI Configuration Access.
pub const VIRTIO_PCI_CAP_PCI_CFG: u8 = 5;

// ---------------------------------------------------------------------------
// Virtio PCI common config structure field offsets (virtio spec 4.1.4.3)
// ---------------------------------------------------------------------------
// All offsets are relative to the BAR address given by the common config
// capability.

const COMMON_DEVICE_FEATURE_SELECT: usize = 0x00;
const COMMON_DEVICE_FEATURE: usize = 0x04;
const COMMON_DRIVER_FEATURE_SELECT: usize = 0x08;
const COMMON_DRIVER_FEATURE: usize = 0x0C;
const COMMON_MSIX_CONFIG: usize = 0x10;
const COMMON_NUM_QUEUES: usize = 0x12;
const COMMON_DEVICE_STATUS: usize = 0x14;
const COMMON_CONFIG_GENERATION: usize = 0x15;
const COMMON_QUEUE_SELECT: usize = 0x16;
const COMMON_QUEUE_SIZE: usize = 0x18;
const COMMON_QUEUE_MSIX_VECTOR: usize = 0x1A;
const COMMON_QUEUE_ENABLE: usize = 0x1C;
const COMMON_QUEUE_NOTIFY_OFF: usize = 0x1E;
const COMMON_QUEUE_DESC: usize = 0x20;
const COMMON_QUEUE_DRIVER: usize = 0x28; // avail ring
const COMMON_QUEUE_DEVICE: usize = 0x30; // used ring

// ---------------------------------------------------------------------------
// PCI status register bits
// ---------------------------------------------------------------------------

/// PCI Status Register: Capabilities List bit.
const PCI_STATUS_CAPABILITIES: u16 = 1 << 4;

// ---------------------------------------------------------------------------
// Virtio PCI transport
// ---------------------------------------------------------------------------

/// Virtio PCI transport.
///
/// Wraps the memory-mapped addresses of virtio PCI capability structures
/// discovered by scanning the PCI config space.
pub struct VirtioPci {
    /// Base address of the BAR containing the common config structure.
    common_cfg_addr: usize,
    /// Base address of the BAR containing the notify structure.
    notify_addr: usize,
    /// `notify_off_multiplier` from the notification capability (virtio spec
    /// 4.1.4.4).  The queue notify address is:
    /// `notify_addr + queue_notify_off * notify_off_multiplier`.
    notify_off_multiplier: u32,
    /// Base address of the BAR containing the ISR status.
    isr_addr: usize,
    /// Base address of the BAR containing the device-specific config.
    device_cfg_addr: usize,
}

impl VirtioPci {
    /// Discover and construct a Virtio PCI transport from a PCI device address.
    ///
    /// `pci_addr` encodes `(bus << 8) | (device << 3) | function` in the
    /// standard PCI addressing scheme.
    ///
    /// Returns `Err(FsError::NotFound)` if the vendor/device ID does not match
    /// virtio, or `Err(FsError::InvalidInput)` if required capabilities are
    /// missing.
    pub fn new(pci_addr: u16) -> Result<Self, crate::fs::FsError> {
        // Verify this is a virtio device.
        let vendor = pci_config_read16(pci_addr, PCI_VENDOR_ID);
        if vendor != VIRTIO_VENDOR_ID {
            return Err(crate::fs::FsError::NotFound);
        }
        let device = pci_config_read16(pci_addr, PCI_DEVICE_ID);
        if !(VIRTIO_DEVICE_ID_BASE..=VIRTIO_DEVICE_ID_BASE + 0x3F).contains(&device) {
            return Err(crate::fs::FsError::NotFound);
        }

        // Enable PCI bus mastering (COMMAND bit 2) so the device can DMA.
        // Read-modify-write the full dword at COMMAND (0x04) to preserve
        // the upper 16 bits (STATUS register, which contains W1C fields).
        let dword = pci_config_read32(pci_addr, PCI_COMMAND);
        pci_config_write32(pci_addr, PCI_COMMAND, dword | (1 << 2));

        // PCI status register bit 4 indicates capabilities list is present.
        // Virtio devices always have capabilities, but guard against garbage.
        let status = pci_config_read16(pci_addr, PCI_STATUS);
        if status & PCI_STATUS_CAPABILITIES == 0 {
            return Err(crate::fs::FsError::InvalidInput);
        }

        // Walk the PCI capability list.
        let mut cap_ptr = pci_config_read8(pci_addr, PCI_CAPABILITIES_PTR) as u16;

        let mut common_cfg_addr: Option<usize> = None;
        let mut notify_addr: Option<usize> = None;
        let mut notify_off_multiplier: u32 = 0;
        let mut isr_addr: Option<usize> = None;
        let mut device_cfg_addr: Option<usize> = None;

        // Safety limit: PCI capability lists are short (< 256 bytes of config
        // space), so 64 iterations is more than enough.
        for _ in 0..64 {
            if cap_ptr == 0 || cap_ptr >= 256 {
                break;
            }

            let cap_id = pci_config_read8(pci_addr, cap_ptr);

            // Virtio capabilities use vendor-specific capability ID 0x09.
            if cap_id == 0x09 {
                let cfg_type = pci_config_read8(pci_addr, cap_ptr + 3);
                let bar = pci_config_read8(pci_addr, cap_ptr + 4);
                let offset = pci_config_read32(pci_addr, cap_ptr + 8);

                if let Ok(bar_addr) = pci_bar_address(pci_addr, bar) {
                    let pa = bar_addr.wrapping_add(offset as usize);
                    let addr = crate::mm::phys_to_virt(pa);
                    match cfg_type {
                        VIRTIO_PCI_CAP_COMMON_CFG => {
                            common_cfg_addr = Some(addr);
                        }
                        VIRTIO_PCI_CAP_NOTIFY_CFG => {
                            notify_addr = Some(addr);
                            notify_off_multiplier =
                                pci_config_read32(pci_addr, cap_ptr + 16);
                        }
                        VIRTIO_PCI_CAP_ISR_CFG => {
                            isr_addr = Some(addr);
                        }
                        VIRTIO_PCI_CAP_DEVICE_CFG => {
                            device_cfg_addr = Some(addr);
                        }
                        _ => {}
                    }
                }
            }

            // Follow the capability linked list.
            cap_ptr = pci_config_read8(pci_addr, cap_ptr + 1) as u16;
        }

        let common_cfg_addr =
            common_cfg_addr.ok_or(crate::fs::FsError::InvalidInput)?;
        let notify_addr = notify_addr.ok_or(crate::fs::FsError::InvalidInput)?;
        let isr_addr = isr_addr.ok_or(crate::fs::FsError::InvalidInput)?;

        Ok(Self {
            common_cfg_addr,
            notify_addr,
            notify_off_multiplier,
            isr_addr,
            device_cfg_addr: device_cfg_addr.unwrap_or(0),
        })
    }

    // -----------------------------------------------------------------------
    // Common config access helpers (memory-mapped, same pattern as MMIO)
    // -----------------------------------------------------------------------

    /// Read a 32-bit field from the common config structure.
    #[inline]
    fn common_read32(&self, offset: usize) -> u32 {
        unsafe {
            (self.common_cfg_addr as *const u32)
                .byte_add(offset)
                .read_volatile()
        }
    }

    /// Write a 32-bit field to the common config structure.
    #[inline]
    fn common_write32(&self, offset: usize, value: u32) {
        unsafe {
            (self.common_cfg_addr as *mut u32)
                .byte_add(offset)
                .write_volatile(value);
        }
    }

    /// Read a 16-bit field from the common config structure.
    #[inline]
    fn common_read16(&self, offset: usize) -> u16 {
        unsafe {
            (self.common_cfg_addr as *const u16)
                .byte_add(offset)
                .read_volatile()
        }
    }

    /// Write a 16-bit field to the common config structure.
    #[inline]
    fn common_write16(&self, offset: usize, value: u16) {
        unsafe {
            (self.common_cfg_addr as *mut u16)
                .byte_add(offset)
                .write_volatile(value);
        }
    }

    /// Read an 8-bit field from the common config structure.
    #[inline]
    fn common_read8(&self, offset: usize) -> u8 {
        unsafe {
            (self.common_cfg_addr as *const u8)
                .byte_add(offset)
                .read_volatile()
        }
    }

    /// Write a 64-bit field to the common config structure.
    #[inline]
    fn common_write64(&self, offset: usize, value: u64) {
        unsafe {
            (self.common_cfg_addr as *mut u64)
                .byte_add(offset)
                .write_volatile(value);
        }
    }
}

// ---------------------------------------------------------------------------
// VirtioTransport implementation
// ---------------------------------------------------------------------------

impl VirtioTransport for VirtioPci {
    fn version(&self) -> u32 { 2 }
    fn set_version(&mut self, _v: u32) {}
    fn status(&self) -> u32 {
        u32::from(self.common_read8(COMMON_DEVICE_STATUS))
    }

    fn set_status(&mut self, status: u32) {
        // Status is an 8-bit register; write only the low byte.
        self.common_write32(COMMON_DEVICE_STATUS, status);
    }

    fn add_status(&mut self, status: u32) {
        let current = self.status();
        self.set_status(current | status);
    }

    fn device_features(&self) -> u64 {
        // Select page 0 (low 32 bits).
        self.common_write32(COMMON_DEVICE_FEATURE_SELECT, 0);
        let low = self.common_read32(COMMON_DEVICE_FEATURE);

        // Select page 1 (high 32 bits).
        self.common_write32(COMMON_DEVICE_FEATURE_SELECT, 1);
        let high = self.common_read32(COMMON_DEVICE_FEATURE);

        u64::from(high) << 32 | u64::from(low)
    }

    fn set_driver_features(&mut self, features: u64) {
        // Write low 32 bits to page 0.
        self.common_write32(COMMON_DRIVER_FEATURE_SELECT, 0);
        self.common_write32(COMMON_DRIVER_FEATURE, features as u32);

        // Write high 32 bits to page 1.
        self.common_write32(COMMON_DRIVER_FEATURE_SELECT, 1);
        self.common_write32(COMMON_DRIVER_FEATURE, (features >> 32) as u32);
    }

    fn queue_select(&mut self, queue: u16) {
        self.common_write16(COMMON_QUEUE_SELECT, queue);
    }

    fn queue_notify(&mut self, queue: u16) {
        // Look up the notify offset for this queue from common config.
        self.common_write16(COMMON_QUEUE_SELECT, queue);
        let notify_off = self.common_read16(COMMON_QUEUE_NOTIFY_OFF);

        let addr = self.notify_addr
            + (notify_off as usize) * (self.notify_off_multiplier as usize);
        unsafe {
            (addr as *mut u16).write_volatile(queue);
        }
    }

    fn queue_size(&self) -> u16 {
        self.common_read16(COMMON_QUEUE_SIZE)
    }

    fn set_queue_size(&mut self, size: u16) {
        self.common_write16(COMMON_QUEUE_SIZE, size);
    }

    fn queue_set_desc_addr(&mut self, addr: u64) {
        self.common_write64(COMMON_QUEUE_DESC, addr);
    }

    fn queue_set_avail_addr(&mut self, addr: u64) {
        self.common_write64(COMMON_QUEUE_DRIVER, addr);
    }

    fn queue_set_used_addr(&mut self, addr: u64) {
        self.common_write64(COMMON_QUEUE_DEVICE, addr);
    }

    fn queue_enable(&mut self) {
        self.common_write16(COMMON_QUEUE_ENABLE, 1);
    }

    fn queue_ready(&self) -> bool {
        self.common_read16(COMMON_QUEUE_ENABLE) != 0
    }

    fn queue_set_pfn_legacy(&mut self, _pfn: u32) {}

    fn read_device_config(&self, offset: usize) -> u32 {
        if self.device_cfg_addr == 0 {
            return 0;
        }
        unsafe {
            (self.device_cfg_addr as *const u32)
                .byte_add(offset)
                .read_volatile()
        }
    }

    fn write_device_config(&mut self, offset: usize, value: u32) {
        if self.device_cfg_addr == 0 {
            return;
        }
        unsafe {
            (self.device_cfg_addr as *mut u32)
                .byte_add(offset)
                .write_volatile(value);
        }
    }
}

// ---------------------------------------------------------------------------
// PCI Configuration Space access via x86_64 port I/O
// ---------------------------------------------------------------------------

/// Build a PCI Configuration Address dword for the given `(bus, dev, func,
/// register)` tuple.
#[inline]
fn pci_address(bus_dev_fn: u16, register: u16) -> u32 {
    // Bit 31    = enable
    // Bits 23:16 = bus (from bus_dev_fn bits 15:8)
    // Bits 15:11 = device (from bus_dev_fn bits 7:3)
    // Bits 10:8  = function (from bus_dev_fn bits 2:0)
    // Bits 7:2   = register (dword-aligned)
    0x8000_0000
        | (u32::from(bus_dev_fn & 0xFF00) << 8)
        | (u32::from(bus_dev_fn & 0x00FF) << 8)
        | (u32::from(register) & 0xFC)
}

/// Read a 32-bit value from PCI config space.
#[inline]
fn pci_config_read32(bus_dev_fn: u16, register: u16) -> u32 {
    use x86_64::instructions::port::Port;
    let addr = pci_address(bus_dev_fn, register);
    unsafe {
        Port::<u32>::new(PCI_CONFIG_ADDR).write(addr);
        Port::<u32>::new(PCI_CONFIG_DATA).read()
    }
}

/// Read a 16-bit value from PCI config space.
#[inline]
pub fn pci_config_read16(bus_dev_fn: u16, register: u16) -> u16 {
    let aligned = pci_config_read32(bus_dev_fn, register);
    let byte_offset = (register & 3) * 8;
    ((aligned >> byte_offset) & 0xFFFF) as u16
}

/// Read an 8-bit value from PCI config space.
#[inline]
fn pci_config_read8(bus_dev_fn: u16, register: u16) -> u8 {
    let aligned = pci_config_read32(bus_dev_fn, register);
    let byte_offset = (register & 3) * 8;
    ((aligned >> byte_offset) & 0xFF) as u8
}

/// Write a 32-bit value to PCI config space.
#[inline]
fn pci_config_write32(bus_dev_fn: u16, register: u16, value: u32) {
    use x86_64::instructions::port::Port;
    let addr = pci_address(bus_dev_fn, register);
    unsafe {
        Port::<u32>::new(PCI_CONFIG_ADDR).write(addr);
        Port::<u32>::new(PCI_CONFIG_DATA).write(value);
    }
}

/// Read the base address of a PCI BAR.
///
/// Supports both 32-bit and 64-bit memory BARs.  64-bit BARs span two
/// consecutive BAR registers (low 32 bits in BAR[n], high 32 bits in
/// BAR[n+1]).  Returns `Err` for I/O BARs.
fn pci_bar_address(bus_dev_fn: u16, bar_index: u8) -> Result<usize, crate::fs::FsError> {
    if bar_index > 5 {
        return Err(crate::fs::FsError::InvalidInput);
    }
    let register = PCI_BAR0 + u16::from(bar_index) * 4;
    let raw = pci_config_read32(bus_dev_fn, register);

    // Bit 0: 0 = memory, 1 = I/O.
    if raw & 1 != 0 {
        return Err(crate::fs::FsError::InvalidInput);
    }

    // Bits [2:1]: memory type.  0b00 = 32-bit, 0b10 = 64-bit.
    let mem_type = (raw >> 1) & 0x3;
    if mem_type == 0b10 {
        if bar_index >= 5 {
            return Err(crate::fs::FsError::InvalidInput);
        }
        let lo = (raw & !0xF) as usize;
        let hi = pci_config_read32(bus_dev_fn, register + 4) as usize;
        return Ok(lo | (hi << 32));
    }

    // 32-bit BAR: mask off the type/flag bits (low 4 bits).
    Ok((raw & !0xF) as usize)
}
