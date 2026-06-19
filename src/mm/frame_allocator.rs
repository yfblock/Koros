//! Physical frame allocator.
//!
//! Manages free physical 4 KiB frames using a sorted list of region descriptors.
//! Regions are added during `mm::init()` after parsing FDT or Multiboot memory maps.
//! The allocator is lock-free single-core for now (no MP safety).

use core::cmp::Ordering;

/// Number of region descriptors in the free list.
const MAX_REGIONS: usize = 32;

/// A free memory region: half-open range `[start, end)` in physical address bytes.
#[derive(Clone, Copy, Debug)]
struct Region {
    start: usize,
    end: usize,
}

/// Physical-frame allocator.
///
/// Maintains a sorted, non-overlapping, merged list of free physical-memory
/// regions.  Allocates by first-fit: scans the list for the first region large
/// enough to satisfy a 4 KiB frame request, splits it, and returns the lowest
/// available frame.
pub struct FrameAllocator {
    regions: [Region; MAX_REGIONS],
    num: usize,
}

impl FrameAllocator {
    const EMPTY: Region = Region { start: 0, end: 0 };
    const PAGE: usize = 4096;

    /// Create an empty allocator.
    pub const fn new() -> Self {
        Self {
            regions: [Self::EMPTY; MAX_REGIONS],
            num: 0,
        }
    }

    /// Add a free physical memory region `[start, end)`.
    ///
    /// Both ends are byte addresses and must be page-aligned.
    /// Adjacent or overlapping regions are merged automatically.
    pub fn add_region(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        assert!(start % Self::PAGE == 0, "start {:#x} not page-aligned", start);
        assert!(end % Self::PAGE == 0, "end {:#x} not page-aligned", end);

        let new = Region { start, end };

        // Find insertion point (sorted by start address).
        let mut i = 0;
        while i < self.num {
            match self.regions[i].start.cmp(&new.start) {
                Ordering::Less if self.regions[i].end >= new.start => {
                    // Overlap / adjacent: merge into existing.
                    if new.end > self.regions[i].end {
                        self.regions[i].end = new.end;
                        try_merge(&mut self.regions, &mut self.num, i);
                    }
                    return;
                }
                Ordering::Equal | Ordering::Greater => break,
                _ => {}
            }
            i += 1;
        }

        if i < self.num && self.regions[i].start == new.end {
            // Adjacent to the next region — merge backwards into us.
            // We'll insert `new` at `i` and it'll absorb regions[i].
            self.insert(i, new);
            if self.regions[i].end == self.regions[i + 1].start {
                self.regions[i].end = self.regions[i + 1].end;
                self.remove(i + 1);
            }
            return;
        }

        if i > 0 && self.regions[i - 1].end == new.start {
            // Adjacent to the previous region: extend it.
            self.regions[i - 1].end = new.end;
            try_merge(&mut self.regions, &mut self.num, i - 1);
            return;
        }

        self.insert(i, new);
    }

    /// Allocate one 4 KiB physical frame.
    ///
    /// Returns `Some(physical_address)` or `None` if out of memory.
    pub fn alloc(&mut self) -> Option<usize> {
        if self.num == 0 {
            return None;
        }

        let frame = self.regions[0].start;
        self.regions[0].start += Self::PAGE;

        if self.regions[0].start >= self.regions[0].end {
            self.remove(0);
        }

        Some(frame)
    }

    /// Free a 4 KiB frame previously returned by [`alloc`].
    ///
    /// `addr` must be 4 KiB-aligned and must not already be free.
    pub fn free(&mut self, addr: usize) {
        assert!(addr % Self::PAGE == 0, "addr {:#x} not page-aligned", addr);
        self.add_region(addr, addr + Self::PAGE);
    }

    /// Punch out (reserve) a region, removing it from the free pool.
    ///
    /// This is used to exclude the kernel image from allocatable memory.
    pub fn reserve(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        let mut i = 0;
        while i < self.num {
            let r = &self.regions[i];
            if r.end <= start {
                i += 1;
                continue;
            }
            if r.start >= end {
                break;
            }
            // Overlap: region[i] overlaps [start, end).
            if r.start < start && r.end > end {
                // Split: region[i] becomes left part, insert right part.
                let right = Region { start: end, end: r.end };
                self.regions[i].end = start;
                self.insert(i + 1, right);
                break;
            }
            if r.start < start {
                self.regions[i].end = start;
                i += 1;
            } else if r.end > end {
                self.regions[i].start = end;
                break;
            } else {
                // Fully consumed.
                self.remove(i);
                // Don't increment — the next region shifted into position i.
            }
        }
    }

    /// Number of tracked free frames.
    pub fn available_frames(&self) -> usize {
        let mut total = 0;
        for r in &self.regions[..self.num] {
            total += (r.end - r.start) / Self::PAGE;
        }
        total
    }

    // --- helpers -----------------------------------------------------------

    fn insert(&mut self, idx: usize, r: Region) {
        assert!(self.num < MAX_REGIONS, "FrameAllocator region OOM");
        // shift right
        let src = idx..self.num;
        self.regions.copy_within(src, idx + 1);
        self.regions[idx] = r;
        self.num += 1;
    }

    fn remove(&mut self, idx: usize) {
        let src = (idx + 1)..self.num;
        self.regions.copy_within(src, idx);
        self.num -= 1;
    }
}

// ---------------------------------------------------------------------------
// Global allocator instance — accessed from mm::init() and driver code.
// ---------------------------------------------------------------------------

use core::cell::UnsafeCell;

pub(crate) struct Allocator(pub(crate) UnsafeCell<FrameAllocator>);

// SAFETY: Only initialised once (in `mm::init`) and accessed from a single core.
unsafe impl Sync for Allocator {}

/// Singleton frame allocator.
pub static ALLOCATOR: Allocator = Allocator(UnsafeCell::new(FrameAllocator::new()));

/// Allocate a single 4 KiB physical frame.
pub fn alloc_frame() -> Option<usize> {
    unsafe { (*ALLOCATOR.0.get()).alloc() }
}

pub unsafe fn free_frame(addr: usize) {
    unsafe { (*ALLOCATOR.0.get()).free(addr) }
}

pub fn available_frames() -> usize {
    unsafe { (*ALLOCATOR.0.get()).available_frames() }
}

/// Try to merge region at `i` with `i+1` if adjacent.
fn try_merge(regions: &mut [Region; MAX_REGIONS], num: &mut usize, i: usize) {
    if i + 1 < *num && regions[i].end >= regions[i + 1].start {
        if regions[i + 1].end > regions[i].end {
            regions[i].end = regions[i + 1].end;
        }
        // shift left
        let src = (i + 2)..*num;
        regions.copy_within(src, i + 1);
        *num -= 1;
    }
}
