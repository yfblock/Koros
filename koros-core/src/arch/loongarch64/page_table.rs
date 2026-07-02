//! LoongArch64 page-table stubs.
//!
//! Boot does not enable paging yet; dynamic maps are unavailable.

use crate::mm::{MapError, MapSize, MappingFlags};

pub const TEST_VA_4K: usize = 0;
pub const TEST_VA_2M: usize = 0;

pub fn init() {}

pub fn dynamic_maps_supported() -> bool {
    false
}

pub fn map(_vaddr: usize, _paddr: usize, _flags: MappingFlags, _size: MapSize) -> Result<(), MapError> {
    Err(MapError::Unsupported)
}

pub fn translate(_vaddr: usize) -> Option<usize> {
    None
}
