//! LoongArch64 page-table stubs.
//!
//! Boot does not enable paging yet; dynamic maps are unavailable.

use kor::{MapError, MapSize, MappingFlags};
use crate::AddrSpace;

/// Kernel direct-map offset (matches the boot table built by the trampoline).
const KERNEL_OFFSET: usize = 0;

/// Zero one page at `pa` through the kernel direct map.
///
/// # Safety
/// `pa` must be a page-aligned frame available for exclusive write.
unsafe fn zero_page(pa: usize) {
    let va = phys_to_virt(pa) as *mut u64;
    // SAFETY: caller guarantees exclusive access to the frame.
    unsafe { core::ptr::write_bytes(va, 0, 512) };
}

/// Physical -> kernel direct-map virtual address.
fn phys_to_virt(pa: usize) -> usize {
    pa + KERNEL_OFFSET
}



pub fn init() {}

pub fn map(
    _vaddr: usize,
    _paddr: usize,
    _flags: MappingFlags,
    _size: MapSize,
) -> Result<(), MapError> {
    Err(MapError::Unsupported)
}

pub fn translate(_vaddr: usize) -> Option<usize> {
    None
}

/// Placeholder encoding items so the arch-neutral engine links; never used
/// at runtime (every facade entry point checks `PAGING_SUPPORTED` first).
impl crate::AddrSpace {
    pub const LEVELS: usize = 1;
    pub const PAGING_SUPPORTED: bool = false;

    pub fn table_entry(_table_phys: usize) -> usize {
        0
    }
    pub fn leaf_entry(_phys: usize, _flags: MappingFlags, _level: usize) -> usize {
        0
    }
    pub fn entry_present(_entry: usize) -> bool {
        false
    }
    pub fn entry_is_leaf(_entry: usize, _level: usize) -> bool {
        false
    }
    pub fn entry_phys(_entry: usize, _level: usize) -> usize {
        0
    }
    pub fn entry_flags(_entry: usize) -> MappingFlags {
        MappingFlags::R
    }
    pub fn flush_tlb(_va: Option<usize>) {}
}

pub fn unmap(_vaddr: usize, _size: MapSize) -> Result<usize, MapError> {
    Err(MapError::Unsupported)
}

/// Identity mapping: the "root" is a sentinel, never switched.
pub fn root() -> usize {
    0
}

/// # Safety
/// Never called: paging cannot be enabled on this architecture.
pub unsafe fn activate(_root: usize) {}

pub fn clone_active() -> Result<usize, MapError> {
    Err(MapError::Unsupported)
}

pub fn new_user() -> Result<usize, MapError> {
    Err(MapError::Unsupported)
}

/// # Safety
/// Never called: no address spaces exist.
pub unsafe fn destroy(_root: usize) {}

pub fn split_identity() {}

impl AddrSpace {
    /// Physical -> kernel direct-map virtual address (identity here).
    pub fn phys_to_virt(pa: usize) -> usize {
        phys_to_virt(pa)
    }

    /// Zero one page at `pa` through the direct map.
    ///
    /// # Safety
    /// `pa` must be a page-aligned frame available for exclusive write.
    pub unsafe fn zero_page(pa: usize) {
        // SAFETY: forwarded under the same contract.
        unsafe { zero_page(pa) }
    }
}
