//! Virtio device support, built on the external `virtio-drivers` crate.
//!
//! [`vd`] implements the crate's `Hal` against Koros' memory manager and wraps
//! its `VirtIOBlk` (over the MMIO or PCI transport) in the kernel's
//! [`BlockDevice`](crate::drivers::block::BlockDevice) trait.

pub mod vd;
