//! Generic radix page-table engine (CortenMM, teaching-kernel scale).
//!
//! One address space is one radix tree of page-table pages plus the
//! per-page descriptors ([`crate::descriptor`]).  All mutations go through
//! the transactional interface: `AddrSpace::lock(range)` returns an
//! [`RCursor`]; `query`/`map`/`mark`/`unmap` execute atomically inside the
//! transaction; dropping the cursor releases the locks in reverse order.
//!
//! Concurrency: this implements the CortenMM<sub>rw</sub> shape simplified
//! to exclusive per-page locks (a covering lock set held for the whole
//! transaction), which serializes overlapping transactions.  The paper's
//! RCU-based advanced protocol and TLB-shootdown optimizations are out of
//! scope at this scale.

use crate::arch::current;
use crate::descriptor::{PT_ENTRIES, PageStatus, descriptor_of};
use kor::frame::{PAGE_SIZE, alloc_frame, free_frames};
use kor::mapping::{MapError, MapSize, MappingFlags};

/// Bits of virtual address consumed per level (512-entry tables).
const LEVEL_SHIFT: usize = 9;
/// Bits below the level indexes (4 KiB base pages).
const PAGE_SHIFT: usize = 12;

/// Page-table index of `va` at `level` (0 = leaf level).
///
/// Identical on every supported format: 9-bit slices above the base page.
fn table_index(va: usize, level: usize) -> usize {
    (va >> (PAGE_SHIFT + LEVEL_SHIFT * level)) & (PT_ENTRIES - 1)
}

/// Bytes covered by one mapping of `size`.
fn map_size_bytes(size: MapSize) -> usize {
    match size {
        MapSize::Page4K => PAGE_SIZE,
        MapSize::Page2M => 2 * 1024 * 1024,
    }
}

/// Span of one entry at `level` — the leaf size there (4 KiB base, 2 MiB,
/// 1 GiB, ...), which on every supported format is also the region a child
/// table covers.
fn span(level: usize) -> usize {
    1usize << (PAGE_SHIFT + LEVEL_SHIFT * level)
}

/// Create a mutable view of the table page at physical address `phys`.
///
/// # Safety
/// The page must be a live page-table page and accesses must be serialized
/// by its descriptor lock (or the page must be otherwise exclusive).
unsafe fn table_mut(phys: usize) -> &'static mut [usize; PT_ENTRIES] {
    // SAFETY: caller guarantees a live, exclusively-accessed table page.
    unsafe { &mut *(current::phys_to_virt(phys) as *mut [usize; PT_ENTRIES]) }
}

/// One address space: the physical root of its page-table tree.
pub struct AddrSpace {
    root_phys: usize,
}

impl AddrSpace {
    /// Allocate and zero a fresh (empty) root table.
    pub fn new() -> Result<Self, MapError> {
        let root = alloc_frame().ok_or(MapError::OutOfMemory)?;
        // SAFETY: the fresh frame is a zeroed table page we own.
        // SAFETY: fresh frame owned by this call.
        unsafe { current::zero_page(root) };
        Ok(Self { root_phys: root })
    }

    /// Wrap an existing root (e.g. the boot page table).
    ///
    /// # Safety
    /// `root` must be a valid page-table root for the installed format that
    /// nothing else mutates concurrently.
    pub unsafe fn from_root(root_phys: usize) -> Self {
        Self { root_phys }
    }

    /// Physical address of the root table.
    pub fn root(&self) -> usize {
        self.root_phys
    }

    /// Make this address space the active one on the current CPU and
    /// invalidate stale translations.
    ///
    /// # Safety
    /// The root must be a valid, page-aligned table containing the kernel
    /// mappings required by the current execution context.  This is the
    /// method form of [`crate::activate_address_space`].
    pub unsafe fn activate(&self) {
        // SAFETY: forwarded to the arch backend under the same contract.
        unsafe { crate::arch::current::activate(self.root_phys) };
    }

    /// Begin a transaction on `[start, end)`.
    ///
    /// Locks every page-table page from the root down through the pages
    /// fully covering the range; the locks are held until the returned
    /// cursor is dropped.
    pub fn lock(&self, range: core::ops::Range<usize>) -> RCursor<'_> {
        let mut cursor = RCursor {
            space: self,
            locked: alloc::vec::Vec::new(),
        };
        cursor.lock_range(range);
        cursor
    }

    /// Map through a transaction on this address space.
    pub fn map(
        &self,
        va: usize,
        phys: usize,
        flags: MappingFlags,
        size: MapSize,
    ) -> Result<(), MapError> {
        let mut txn = self.lock(va..va + map_size_bytes(size));
        txn.map(va, phys, flags, size)
    }

    /// Translate: the leaf phys plus the in-page offset a coarse PTE does
    /// not carry (e.g. Sv39 megapage PA[20:12] come from the VA).
    pub fn translate(&self, va: usize) -> Option<usize> {
        let txn = self.lock(va..va + PAGE_SIZE);
        match txn.query(va) {
            PageStatus::Mapped {
                phys, granularity, ..
            } => Some(phys + va % granularity),
            _ => None,
        }
    }

    /// Unmap through a transaction; returns the mapped physical address.
    pub fn unmap(&self, va: usize, size: MapSize) -> Result<usize, MapError> {
        let Some(paddr) = self.translate(va) else {
            return Err(MapError::BlockedByExistingMapping);
        };
        let mut txn = self.lock(va..va + map_size_bytes(size));
        txn.unmap(va..va + map_size_bytes(size)).map(|_| paddr)
    }

    /// Tear the whole address space down, freeing every table page (data
    /// pages are the owner's business) and flushing the TLB.
    ///
    /// # Safety
    /// The address space must not be active on any CPU.
    pub unsafe fn destroy(self) {
        let _root_guard = descriptor_of(self.root_phys).acquire();
        // SAFETY: root is locked and exclusively ours.
        unsafe { Self::free_subtree(self.root_phys, Self::LEVELS - 1) };
        drop(_root_guard);
        free_frames(self.root_phys, 1);
        Self::flush_tlb(None);
    }

    /// Free a whole table subtree rooted at `phys` (excluding data pages).
    ///
    /// # Safety
    /// `phys` is a live table page at `level`; its descriptor is locked.
    unsafe fn free_subtree(phys: usize, level: usize) {
        let table = unsafe { table_mut(phys) };
        for index in 0..PT_ENTRIES {
            let entry = table[index];
            if !Self::entry_present(entry) {
                continue;
            }
            if level > 0 && !Self::entry_is_leaf(entry, level) {
                let child = Self::entry_phys(entry, level);
                let _guard = descriptor_of(child).acquire();
                // SAFETY: child is locked and unreachable after this pass.
                unsafe { Self::free_subtree(child, level - 1) };
                drop(_guard);
                free_frames(child, 1);
            }
        }
        // SAFETY: the page (and its subtree) is now unreachable.
        unsafe { descriptor_of(phys).drop_metadata() };
    }
}

/// Transaction cursor over a locked range of an [`AddrSpace`].
pub struct RCursor<'a> {
    space: &'a AddrSpace,
    /// Table pages locked by this transaction, root first, each with its
    /// RAII guard.  Dropping a guard releases the page; the custom `Drop`
    /// pops from the back so release happens in reverse acquisition order
    /// (CortenMM's unlock protocol).
    locked: alloc::vec::Vec<(usize, spin::MutexGuard<'static, ()>)>,
}

impl Drop for RCursor<'_> {
    fn drop(&mut self) {
        // Reverse-order release, as in CortenMM's unlock: popping from the
        // back drops each guard (and thus unlocks its page) leaf-first.
        while self.locked.pop().is_some() {}
    }
}

impl RCursor<'_> {
    /// Lock `page`'s descriptor unless this transaction already holds it.
    fn lock_page(&mut self, page: usize) {
        if !self.locked.iter().any(|(held, _)| *held == page) {
            let guard = descriptor_of(page).acquire();
            self.locked.push((page, guard));
        }
    }

    fn lock_range(&mut self, range: core::ops::Range<usize>) {
        let mut page = self.space.root_phys;
        self.lock_page(page);
        let mut level = AddrSpace::LEVELS - 1;
        loop {
            if level == 0 {
                break;
            }
            let table = unsafe { table_mut(page) };
            let index = table_index(range.start, level);
            let entry = table[index];
            let child_span = span(level - 1);
            // Descend only while one child page covers the whole range.
            let covered = range.end - 1 < (range.start / child_span + 1) * child_span;
            if !AddrSpace::entry_present(entry)
                || AddrSpace::entry_is_leaf(entry, level)
                || !covered
            {
                break;
            }
            page = AddrSpace::entry_phys(entry, level);
            self.lock_page(page);
            level -= 1;
        }
    }

    /// Status of the virtual page at `va`: the hardware mapping when
    /// present, else the deepest per-PTE metadata (inherited downwards).
    pub fn query(&self, va: usize) -> PageStatus {
        let mut inherited = PageStatus::Invalid;
        let mut page = self.space.root_phys;
        let mut level = AddrSpace::LEVELS - 1;
        loop {
            let table = unsafe { table_mut(page) };
            let index = table_index(va, level);
            let entry = table[index];
            // SAFETY: the transaction holds this page's descriptor lock.
            let status = unsafe { descriptor_of(page).status(index) };
            if status != PageStatus::Invalid {
                inherited = status;
            }
            if !AddrSpace::entry_present(entry) {
                return inherited;
            }
            if AddrSpace::entry_is_leaf(entry, level) || level == 0 {
                return PageStatus::Mapped {
                    phys: AddrSpace::entry_phys(entry, level),
                    flags: AddrSpace::entry_flags(entry),
                    granularity: span(level),
                };
            }
            page = AddrSpace::entry_phys(entry, level);
            level -= 1;
        }
    }

    /// Map `size` at `va` to `phys`.
    ///
    /// Splits an existing larger leaf covering `va` on demand.  Fails with
    /// [`MapError::BlockedByExistingMapping`] if a mapping already exists
    /// at the target granularity.
    pub fn map(
        &mut self,
        va: usize,
        phys: usize,
        flags: MappingFlags,
        size: MapSize,
    ) -> Result<(), MapError> {
        let target_level = match size {
            MapSize::Page4K => 0,
            MapSize::Page2M => 1,
        };
        let granularity = span(target_level);
        if va % granularity != 0 || phys % granularity != 0 {
            return Err(MapError::Unsupported);
        }
        let mut page = self.space.root_phys;
        let mut level = AddrSpace::LEVELS - 1;
        loop {
            let table = unsafe { table_mut(page) };
            let index = table_index(va, level);
            let entry = table[index];
            if level == target_level {
                if AddrSpace::entry_present(entry) {
                    // A hardware leaf whose metadata says `Mapped` is an
                    // explicit mapping — refuse to overwrite it.  A leaf
                    // with `Invalid` metadata is filler created by a split
                    // (or a boot-table leaf) and may be refined.
                    // SAFETY: this transaction holds the page's lock.
                    let refinable =
                        unsafe { descriptor_of(page).status(index) } == PageStatus::Invalid;
                    if !refinable {
                        return Err(MapError::BlockedByExistingMapping);
                    }
                }
                table[index] = AddrSpace::leaf_entry(phys, flags, level);
                // SAFETY: this transaction holds the page's lock.
                unsafe {
                    descriptor_of(page).set_status(
                        index,
                        PageStatus::Mapped {
                            phys,
                            flags,
                            granularity,
                        },
                    )
                };
                AddrSpace::flush_tlb(Some(va));
                return Ok(());
            }
            if !AddrSpace::entry_present(entry) {
                let child = alloc_frame().ok_or(MapError::OutOfMemory)?;
                // SAFETY: fresh frame owned by this transaction.
                // SAFETY: fresh frame owned by this transaction.
                unsafe { current::zero_page(child) };
                table[index] = AddrSpace::table_entry(child);
                self.lock_page(child);
                page = child;
            } else if AddrSpace::entry_is_leaf(entry, level) {
                // Split the larger leaf so finer pages can be mapped inside.
                self.split_leaf(page, index, level)?;
                page = AddrSpace::entry_phys(table[index], level);
            } else {
                page = AddrSpace::entry_phys(entry, level);
                self.lock_page(page);
            }
            level -= 1;
        }
    }

    /// Replace the leaf at `page[index]` (level `level`) with a table of
    /// same-permission children covering the identical region.
    fn split_leaf(&mut self, page: usize, index: usize, level: usize) -> Result<(), MapError> {
        let old_entry = unsafe { table_mut(page) }[index];
        let old_phys = AddrSpace::entry_phys(old_entry, level);
        let old_flags = AddrSpace::entry_flags(old_entry);
        // SAFETY: this transaction holds the page's lock.

        let child = alloc_frame().ok_or(MapError::OutOfMemory)?;
        // SAFETY: fresh frame owned by this transaction.
        // SAFETY: fresh frame owned by this transaction.
        unsafe { current::zero_page(child) };
        let child_table = unsafe { table_mut(child) };
        let child_span = span(level - 1);
        for i in 0..PT_ENTRIES {
            child_table[i] = AddrSpace::leaf_entry(old_phys + i * child_span, old_flags, level - 1);
        }
        let parent_table = unsafe { table_mut(page) };
        parent_table[index] = AddrSpace::table_entry(child);
        // Split filler is metadata-`Invalid`: the hardware leaves carry the
        // old mapping (query reads hardware at leaf level), and finer maps
        // may refine them.  Explicit maps always mark `Mapped`.
        // SAFETY: this transaction holds the page's lock.
        unsafe { descriptor_of(page).set_status(index, PageStatus::Invalid) };
        self.lock_page(child);
        Ok(())
    }

    /// Virtually allocate `[start, end)` with `flags` (on-demand paging):
    /// records the status beside the page table without touching hardware
    /// mappings; page faults later turn `Allocated` into real mappings.
    pub fn mark(&mut self, range: core::ops::Range<usize>, flags: MappingFlags) {
        let root = self.space.root_phys;
        let levels = AddrSpace::LEVELS - 1;
        self.mark_in(root, 0, levels, range, flags);
    }

    fn mark_in(
        &mut self,
        page: usize,
        base: usize,
        level: usize,
        range: core::ops::Range<usize>,
        flags: MappingFlags,
    ) {
        let table = unsafe { table_mut(page) };
        let span = span(level);
        let first = table_index(range.start, level);
        let last = table_index(range.end - 1, level);
        for index in first..=last {
            let entry_start = base + index * span;
            let entry_end = entry_start + span;
            let covered = range.start <= entry_start && entry_end <= range.end;
            let entry = table[index];
            if covered {
                // Whole entry inside the range: mark at this granularity
                // (upper levels stand in for large uniform regions).
                // SAFETY: this transaction holds the page's lock.
                unsafe { descriptor_of(page).set_status(index, PageStatus::Allocated(flags)) };
            } else if level > 0
                && AddrSpace::entry_present(entry)
                && !AddrSpace::entry_is_leaf(entry, level)
            {
                // Partially covered: descend into an existing finer table.
                let child = AddrSpace::entry_phys(entry, level);
                let lo = range.start.max(entry_start);
                let hi = range.end.min(entry_end);
                self.lock_page(child);
                self.mark_in(child, entry_start, level - 1, lo..hi, flags);
            }
        }
    }
    /// to the frame allocator (the root is kept).
    pub fn unmap(&mut self, range: core::ops::Range<usize>) -> Result<usize, MapError> {
        if range.start >= range.end {
            return Err(MapError::Unsupported);
        }
        let mut unmapped = 0usize;
        let root = self.space.root_phys;
        let levels = AddrSpace::LEVELS - 1;
        self.unmap_in(root, 0, levels, range, &mut unmapped)?;
        AddrSpace::flush_tlb(None);
        Ok(unmapped)
    }

    fn unmap_in(
        &mut self,
        page: usize,
        base: usize,
        level: usize,
        range: core::ops::Range<usize>,
        unmapped: &mut usize,
    ) -> Result<(), MapError> {
        let table = unsafe { table_mut(page) };
        let span = span(level);
        let first = table_index(range.start, level);
        let last = table_index(range.end - 1, level);
        for index in first..=last {
            let entry = table[index];
            if !AddrSpace::entry_present(entry) {
                continue;
            }
            let entry_start = base + index * span;
            let entry_end = entry_start + span;
            let fully = range.start <= entry_start && entry_end <= range.end;
            if AddrSpace::entry_is_leaf(entry, level) {
                if fully || level == 0 {
                    table[index] = 0;
                    // SAFETY: this transaction holds the page's lock.
                    unsafe { descriptor_of(page).set_status(index, PageStatus::Invalid) };
                    *unmapped += span / PAGE_SIZE;
                }
            } else if level > 0 {
                let child = AddrSpace::entry_phys(entry, level);
                let lo = range.start.max(entry_start);
                let hi = range.end.min(entry_end);
                self.lock_page(child);
                self.unmap_in(child, entry_start, level - 1, lo..hi, unmapped)?;
                // Free the child when it is now completely empty.
                let child_table = unsafe { table_mut(child) };
                let empty = (0..PT_ENTRIES).all(|i| child_table[i] == 0);
                if empty {
                    let child_desc = descriptor_of(child);
                    // SAFETY: the child is unreachable from now on.
                    unsafe { child_desc.drop_metadata() };
                    free_frames(child, 1);
                    // Dropping the guard releases the freed child's lock.
                    self.locked.retain(|(held, _)| *held != child);
                    table[index] = 0;
                }
            }
        }
        Ok(())
    }
}
