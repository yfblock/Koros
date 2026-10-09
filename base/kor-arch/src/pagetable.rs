//! Basic page-table mapping logic, per arch (uniform free-function surface)
//! plus the shared walk helpers used by kor-mmu's managed engine.
//!
//! kor-arch owns the *mechanism*: boot-table building (the trampoline
//! `boot_mmu_build`/`boot_mmu_activate` hooks live here), active-root
//! switching, raw table walks, PTE construction, fork-style table cloning,
//! and boot identity surgery.  kor-mmu wraps this with per-table-page
//! descriptors and the transactional cursor.

cfg_select! {
    target_arch = "riscv64"      => { pub use crate::riscv64::pagetable::*; }
    target_arch = "x86_64"       => { pub use crate::x86_64::pagetable::*; }
    target_arch = "aarch64"      => { pub use crate::aarch64::pagetable::*; }
}

/// Entries per page-table page (512 x 64-bit PTEs at every level).
pub const PT_ENTRIES: usize = 512;

/// View a table page (physical) through the kernel mapping.
///
/// # Safety
/// The page must be a live table page accessed exclusively (held by its
/// descriptor lock, or otherwise unshared).
pub unsafe fn table_mut(phys: usize) -> &'static mut [usize; PT_ENTRIES] {
    // SAFETY: caller guarantees a live, exclusively-accessed table page.
    unsafe { &mut *(kor::PhysAddr::new(phys).to_ka().raw() as *mut [usize; PT_ENTRIES]) }
}

/// Level-`n` index of `va` (n = 0 is the 4 KiB level).
pub fn table_index(va: usize, level: usize) -> usize {
    kor::VirtAddr::new(va).pn_index(level)
}

/// Bytes spanned by one entry at `level` (n = 0 is the 4 KiB level).
pub fn span(level: usize) -> usize {
    1 << (12 + 9 * level)
}
