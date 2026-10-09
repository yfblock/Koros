//! `kor-mmu` — the managed page-table wrapper over kor-arch (CortenMM,
//! SOSP '25).
//!
//! Layout:
//! - `kor_arch::pagetable`: the basic per-arch mapping logic (raw walks,
//!   PTE construction, boot tables, root switching, table cloning).
//! - [`descriptor`]: per-table-page descriptors (lock + per-PTE metadata).
//! - [`page_table`]: the transactional engine (`AddrSpace` + `RCursor`)
//!   wrapping kor-arch primitives.
//!
//! The active address space is obtained via [`active()`] and operated on
//! through its methods — no per-operation free-function wrappers.
//!
//! Reference: Zhang et al., "CortenMM: Efficient Memory Management with
//! Strong Correctness Guarantees", SOSP 2025.  Differences at this scale:
//! exclusive per-page locks instead of the rw/RCU protocols, no TLB
//! shootdown machinery, no reverse mapping.

#![no_std]

extern crate alloc;

pub mod descriptor;
pub mod page_table;

pub use descriptor::PageStatus;
pub use page_table::{AddrSpace, RCursor};

use kor::mapping::MapError;

/// Install runtime page-table services (e.g. capture the boot root).
pub fn page_table_init() {
    kor_arch::pagetable::init();
}

/// Whether dynamic mappings are supported (false on unpaged arches).
pub fn dynamic_maps_supported() -> bool {
    kor_arch::pagetable::PAGING_SUPPORTED
}

/// The currently active address space.
///
/// # Panics
/// Panics if paging is not supported on this architecture.
pub fn active() -> AddrSpace {
    assert!(
        kor_arch::pagetable::PAGING_SUPPORTED,
        "no page tables on this architecture"
    );
    // SAFETY: the active root is live for the duration of the borrow.
    unsafe { AddrSpace::from_root(kor_arch::pagetable::root()) }
}

/// Physical root of the active address space (0 if paging is off).
pub fn address_space_root() -> usize {
    kor_arch::pagetable::root()
}

/// Activate an address-space root and invalidate stale translations.
///
/// # Safety
/// `root` must be a valid, page-aligned root containing the kernel mappings
/// required by the current execution context.
pub unsafe fn activate_address_space(root: usize) {
    // SAFETY: forwarded under the same contract.
    unsafe { kor_arch::pagetable::activate(root) }
}

/// Clone the active address space, deeply copying user pages.
pub fn clone_address_space() -> Result<usize, MapError> {
    kor_arch::pagetable::clone_active()
}

/// Create an empty user address space retaining kernel mappings.
pub fn new_address_space() -> Result<usize, MapError> {
    kor_arch::pagetable::new_user()
}

/// Destroy an inactive address space, releasing its user pages and tables.
///
/// # Safety
/// `root` must not be active on any CPU.
pub unsafe fn destroy_address_space(root: usize) {
    // SAFETY: forwarded under the same contract.
    unsafe { kor_arch::pagetable::destroy(root) }
}

/// Split the boot identity mapping so finer user pages can be mapped
/// inside it (no-op where the boot layout already allows it).
pub fn split_identity() {
    kor_arch::pagetable::split_identity();
}

// Link-time hooks consumed by kor-sched (per-task address-space roots).
// koros (no MMU) provides its own identity stubs instead of linking kor-mmu.

/// Read the current CPU's active page-table root.
#[unsafe(no_mangle)]
pub extern "C" fn sched_address_space_root() -> usize {
    address_space_root()
}

/// Activate `root` on this CPU.
///
/// # Safety
/// `root` must be a valid, exclusively-owned page-table root.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sched_activate_address_space(root: usize) {
    // SAFETY: forwarded under the same contract.
    unsafe { activate_address_space(root) };
}
