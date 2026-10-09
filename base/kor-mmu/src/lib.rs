//! `kor-mmu` — MMU ownership for the Koros kernels.
//!
//! Following CortenMM (SOSP '25): a single-level abstraction with
//! per-table-page descriptors ([`descriptor`]), a transactional cursor
//! ([`page_table::RCursor`]) with per-arch PTE encoding supplied as inherent
//! associated items of [`AddrSpace`] (compile-time dispatch, no vtables).
//!
//! Layout:
//! - [`arch`]: per-architecture back-ends — PTE encoders as inherent
//!   associated items of `AddrSpace` (statically dispatched), active-space
//!   switching, fork cloning, boot identity surgery.  Selected by `cfg` in
//!   one place ([`arch::current`]).
//! - [`descriptor`]/[`page_table`]: the arch-neutral engine.
//!
//! The free functions below are the page-table
//! methods one-to-one, so composition crates call `kor_mmu::map(...)` etc.
//! directly.
//!
//! Reference: Zhang et al., "CortenMM: Efficient Memory Management with
//! Strong Correctness Guarantees", SOSP 2025.  Differences at this scale:
//! exclusive per-page locks instead of the rw/RCU protocols, no TLB
//! shootdown machinery, no reverse mapping.

#![no_std]

extern crate alloc;

pub mod arch;
pub mod descriptor;
pub mod page_table;

pub use descriptor::PageStatus;
pub use page_table::{AddrSpace, RCursor};

use kor::mapping::{MapError, MapSize, MappingFlags};

/// Install runtime page-table services (e.g. capture the boot root).
pub fn page_table_init() {
    arch::current::init();
}

/// Whether dynamic mappings are supported (false on unpaged arches).
pub fn dynamic_maps_supported() -> bool {
    AddrSpace::PAGING_SUPPORTED
}

/// Map through the generic engine on the active root.
pub fn map(vaddr: usize, paddr: usize, flags: MappingFlags, size: MapSize) -> Result<(), MapError> {
    if !AddrSpace::PAGING_SUPPORTED {
        return Err(MapError::Unsupported);
    }
    // SAFETY: the active root is live for the whole call.
    let space = unsafe { AddrSpace::from_root(arch::current::root()) };
    space.map(vaddr, paddr, flags, size)
}

/// Translate through the generic engine on the active root.
pub fn translate(vaddr: usize) -> Option<usize> {
    if !AddrSpace::PAGING_SUPPORTED {
        return None;
    }
    // SAFETY: read-only walk of the live root inside a transaction.
    let space = unsafe { AddrSpace::from_root(arch::current::root()) };
    space.translate(vaddr)
}

/// Unmap through the generic engine; returns the mapped physical address.
pub fn unmap(vaddr: usize, size: MapSize) -> Result<usize, MapError> {
    if !AddrSpace::PAGING_SUPPORTED {
        return Err(MapError::Unsupported);
    }
    // SAFETY: same contract as `map`.
    let space = unsafe { AddrSpace::from_root(arch::current::root()) };
    space.unmap(vaddr, size)
}

/// Physical root of the active address space.
pub fn address_space_root() -> usize {
    arch::current::root()
}

/// Activate an address-space root and invalidate stale translations.
///
/// # Safety
/// `root` must be a valid, page-aligned root containing the kernel mappings
/// required by the current execution context.
pub unsafe fn activate_address_space(root: usize) {
    // SAFETY: forwarded to the arch backend under the same contract.
    unsafe { arch::current::activate(root) }
}

/// Clone the active address space, deeply copying user pages.
pub fn clone_address_space() -> Result<usize, MapError> {
    arch::current::clone_active()
}

/// Create an empty user address space retaining kernel mappings.
pub fn new_address_space() -> Result<usize, MapError> {
    arch::current::new_user()
}

/// Destroy an inactive address space, releasing its user pages and tables.
///
/// # Safety
/// `root` must not be active on any CPU.
pub unsafe fn destroy_address_space(root: usize) {
    // SAFETY: forwarded to the arch backend under the same contract.
    unsafe { arch::current::destroy(root) }
}

/// Split the boot identity mapping so finer user pages can be mapped
/// inside it (no-op where the boot layout already allows it).
pub fn split_identity() {
    arch::current::split_identity();
}
