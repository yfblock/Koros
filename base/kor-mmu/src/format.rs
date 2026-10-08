//! The hardware page-table format contract (CortenMM's portability trait).
//!
//! CortenMM observes that x86, ARM and RISC-V all use multi-level radix
//! page tables with 64-bit PTEs and hides the remaining differences behind
//! a small trait.  `kor-mmu` follows the same shape: the generic engine in
//! [`crate::page_table`] is arch-neutral, each architecture provides one
//! implementation of [`MmuFormat`] (selected by `arch::current`).

use kor::mapping::MappingFlags;

/// Hardware page-table format of one architecture.
///
/// Assumptions (all supported paging ISAs satisfy them):
/// - multi-level radix tree, 4 KiB base pages, 512 × 64-bit PTEs per table;
/// - software can tell PTE validity / leaf-ness, encode permissions, and
///   derive the physical address from an entry;
/// - a per-address or full TLB flush is available.
#[diagnostic::on_unimplemented(
    message = "this target has no `{Self}` implementation",
    label = "missing `{Self}` impl",
    note = "porting checklist: provide the impl for this architecture/crate"
)]
pub trait MmuFormat: Send + Sync {
    /// Number of levels (Sv39: 3, x86-64/AArch64 4-level: 4).  A method
    /// (not an associated const) so the trait stays dyn-compatible.
    fn levels(&self) -> usize;

    /// Page-table index of `va` at `level` (0 = leaf level).
    fn table_index(&self, va: usize, level: usize) -> usize;

    /// Offset of `va` within the page of `level`'s granularity.
    fn page_offset(&self, va: usize, level: usize) -> usize;

    /// Size in bytes of a leaf mapping allowed at `level` (0 if the level
    /// only holds tables).
    fn leaf_size(&self, level: usize) -> usize;

    /// Encode a non-leaf (table pointer) entry pointing at `table_phys`.
    fn table_entry(&self, table_phys: usize) -> usize;

    /// Encode a leaf entry mapping `leaf_size(level)` bytes at `phys`.
    fn leaf_entry(&self, phys: usize, flags: MappingFlags, level: usize) -> usize;

    /// Whether `entry` holds a valid mapping or table pointer.
    fn entry_present(&self, entry: usize) -> bool;

    /// Whether `entry` is a leaf (huge/block/page) mapping.
    fn entry_is_leaf(&self, entry: usize, level: usize) -> bool;

    /// Physical address referenced by a valid entry.
    fn entry_phys(&self, entry: usize, level: usize) -> usize;

    /// Permission bits decoded from a leaf entry.
    fn entry_flags(&self, entry: usize) -> MappingFlags;

    /// Flush the TLB entry for `va` (all entries when `None`).
    fn flush_tlb(&self, va: Option<usize>);
}
