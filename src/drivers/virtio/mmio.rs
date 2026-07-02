//! Virtio MMIO transport implementation.
//!
//! Implements the VirtioTransport trait for MMIO-mapped virtio devices.
//! Supports both legacy (version 1) and modern (version 2) register layouts.

use super::VirtioTransport;

const MAGIC: usize = 0x000;
const VERSION: usize = 0x004;
const DEVICE_ID: usize = 0x008;
const VENDOR_ID: usize = 0x00c;
const DEVICE_FEATURES: usize = 0x010;
const DEVICE_FEATURES_SEL: usize = 0x014;
const DRIVER_FEATURES: usize = 0x020;
const DRIVER_FEATURES_SEL: usize = 0x024;
const QUEUE_SEL: usize = 0x030;
const QUEUE_NUM_MAX: usize = 0x034;
const QUEUE_NUM: usize = 0x038;
const QUEUE_ALIGN_LEGACY: usize = 0x03c;
const QUEUE_PFN_LEGACY: usize = 0x040;
const QUEUE_READY: usize = 0x044;
const QUEUE_NOTIFY: usize = 0x050;
const INTERRUPT_STATUS: usize = 0x060;
const INTERRUPT_ACK: usize = 0x064;
const STATUS: usize = 0x070;
const QUEUE_DESC_LOW: usize = 0x080;
const QUEUE_DESC_HIGH: usize = 0x084;
const QUEUE_AVAIL_LOW: usize = 0x090;
const QUEUE_AVAIL_HIGH: usize = 0x094;
const QUEUE_USED_LOW: usize = 0x0a0;
const QUEUE_USED_HIGH: usize = 0x0a4;
const CONFIG: usize = 0x100;

pub struct VirtioMmio {
    base: usize,
    version: u32,
}

impl VirtioMmio {
    pub const fn new(base_addr: usize) -> Self {
        Self { base: base_addr, version: 0 }
    }

    #[inline]
    fn read_reg32(&self, offset: usize) -> u32 {
        unsafe { (self.base as *const u32).add(offset / 4).read_volatile() }
    }

    #[inline]
    fn write_reg32(&self, offset: usize, value: u32) {
        unsafe {
            (self.base as *mut u32).add(offset / 4).write_volatile(value);
        }
    }
}

impl VirtioTransport for VirtioMmio {
    fn version(&self) -> u32 {
        self.read_reg32(VERSION)
    }

    fn status(&self) -> u32 {
        self.read_reg32(STATUS)
    }

    fn set_status(&mut self, status: u32) {
        self.write_reg32(STATUS, status);
    }

    fn add_status(&mut self, status: u32) {
        let current = self.status();
        self.set_status(current | status);
    }

    fn device_features(&self) -> u64 {
        self.write_reg32(DEVICE_FEATURES_SEL, 0);
        let low = self.read_reg32(DEVICE_FEATURES);

        if self.version >= 2 {
            self.write_reg32(DEVICE_FEATURES_SEL, 1);
            let high = self.read_reg32(DEVICE_FEATURES);
            u64::from(high) << 32 | u64::from(low)
        } else {
            u64::from(low)
        }
    }

    fn set_driver_features(&mut self, features: u64) {
        if self.version >= 2 {
            self.write_reg32(DRIVER_FEATURES_SEL, 0);
            self.write_reg32(DRIVER_FEATURES, features as u32);
            self.write_reg32(DRIVER_FEATURES_SEL, 1);
            self.write_reg32(DRIVER_FEATURES, (features >> 32) as u32);
        } else {
            self.write_reg32(DRIVER_FEATURES, features as u32);
        }
    }

    fn queue_select(&mut self, queue: u16) {
        self.write_reg32(QUEUE_SEL, u32::from(queue));
    }

    fn queue_notify(&mut self, queue: u16) {
        self.write_reg32(QUEUE_NOTIFY, u32::from(queue));
    }

    fn queue_size(&self) -> u16 {
        self.read_reg32(QUEUE_NUM) as u16
    }

    fn set_queue_size(&mut self, size: u16) {
        self.write_reg32(QUEUE_NUM, u32::from(size));
    }

    fn queue_set_desc_addr(&mut self, addr: u64) {
        if self.version >= 2 {
            self.write_reg32(QUEUE_DESC_LOW, addr as u32);
            self.write_reg32(QUEUE_DESC_HIGH, (addr >> 32) as u32);
        }
    }

    fn queue_set_avail_addr(&mut self, addr: u64) {
        if self.version >= 2 {
            self.write_reg32(QUEUE_AVAIL_LOW, addr as u32);
            self.write_reg32(QUEUE_AVAIL_HIGH, (addr >> 32) as u32);
        }
    }

    fn queue_set_used_addr(&mut self, addr: u64) {
        if self.version >= 2 {
            self.write_reg32(QUEUE_USED_LOW, addr as u32);
            self.write_reg32(QUEUE_USED_HIGH, (addr >> 32) as u32);
        }
    }

    fn queue_enable(&mut self) {
        if self.version >= 2 {
            self.write_reg32(QUEUE_READY, 1);
        }
    }

    fn queue_ready(&self) -> bool {
        if self.version >= 2 {
            self.read_reg32(QUEUE_READY) != 0
        } else {
            self.read_reg32(QUEUE_PFN_LEGACY) != 0
        }
    }

    fn queue_set_pfn_legacy(&mut self, pfn: u32) {
        self.write_reg32(QUEUE_ALIGN_LEGACY, 4096);
        self.write_reg32(QUEUE_PFN_LEGACY, pfn);
    }

    fn read_device_config(&self, offset: usize) -> u32 {
        self.read_reg32(CONFIG + offset)
    }

    fn write_device_config(&mut self, offset: usize, value: u32) {
        self.write_reg32(CONFIG + offset, value);
    }

    fn set_version(&mut self, v: u32) {
        self.version = v;
    }
}
