//! Virtio transport abstraction layer.

pub mod blk;
pub mod mmio;
#[cfg(target_arch = "x86_64")]
pub mod pci;

/// Trait abstracting the virtio transport layer (MMIO or PCI).
///
/// Each method maps to a virtio operation that differs by transport.
/// See virtio spec Section 4.2.2 (MMIO) and Section 4.1 (PCI).
pub trait VirtioTransport: Send + Sync {
    fn version(&self) -> u32;
    fn set_version(&mut self, v: u32);
    fn status(&self) -> u32;
    fn set_status(&mut self, status: u32);
    fn add_status(&mut self, status: u32);
    fn device_features(&self) -> u64;
    fn set_driver_features(&mut self, features: u64);
    fn queue_select(&mut self, queue: u16);
    fn queue_notify(&mut self, queue: u16);
    fn queue_size(&self) -> u16;
    fn set_queue_size(&mut self, size: u16);
    fn queue_set_desc_addr(&mut self, addr: u64);
    fn queue_set_avail_addr(&mut self, addr: u64);
    fn queue_set_used_addr(&mut self, addr: u64);
    fn queue_enable(&mut self);
    fn queue_ready(&self) -> bool;
    fn queue_set_pfn_legacy(&mut self, pfn: u32);
    fn read_device_config(&self, offset: usize) -> u32;
    fn write_device_config(&mut self, offset: usize, value: u32);
}
