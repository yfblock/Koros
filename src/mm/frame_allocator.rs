//! Physical frame allocator — buddy system via `buddy_system_allocator`.
//!
//! Free physical memory is registered during `mm::init()` after parsing FDT or
//! Multiboot memory maps.  Allocations are counted in 4 KiB frames; `count` is
//! rounded up to the next power of two by the underlying buddy allocator.
//!
//! Examples:
//! - `alloc_frames(1)`  — one 4 KiB page
//! - `alloc_frames(512)` — one 2 MiB huge page (512 × 4 KiB, 2 MiB aligned)
//!
//! Single-core only for now (no MP safety on the global instance).

use buddy_system_allocator::FrameAllocator as BuddyFrameAllocator;
use core::cell::UnsafeCell;

/// Size of one physical page / frame.
pub const PAGE_SIZE: usize = 4096;

/// Number of 4 KiB frames in a 2 MiB huge page.
pub const FRAMES_2M: usize = 512;

/// Max buddy order — supports blocks up to 2^(ORDER-1) frames (2 GiB at 4 KiB/frame).
const BUDDY_ORDER: usize = 32;

/// Physical-frame allocator wrapping the buddy crate.
pub struct FrameAllocator {
    inner: BuddyFrameAllocator<BUDDY_ORDER>,
    free_frames: usize,
}

impl FrameAllocator {
    pub const fn new() -> Self {
        Self {
            inner: BuddyFrameAllocator::new(),
            free_frames: 0,
        }
    }

    /// Register free physical memory `[start, end)` (byte addresses).
    ///
    /// Non-page-aligned bounds are rounded inward.
    pub fn add_region(&mut self, start: usize, end: usize) {
        let start = (start + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let end = end & !(PAGE_SIZE - 1);
        if start >= end {
            return;
        }

        let ppn_start = start >> 12;
        let ppn_end = end >> 12;
        self.inner.add_frame(ppn_start, ppn_end);
        self.free_frames += ppn_end - ppn_start;
    }

    /// Allocate `count` contiguous 4 KiB frames (rounded up to a power of two).
    ///
    /// Returns the physical address of the first frame, or `None` if OOM.
    pub fn alloc_frames(&mut self, count: usize) -> Option<usize> {
        let ppn = self.inner.alloc(count)?;
        let allocated = count.next_power_of_two();
        self.free_frames -= allocated;
        Some(ppn << 12)
    }

    /// Free `count` contiguous 4 KiB frames previously returned by [`alloc_frames`].
    ///
    /// `phys` must be page-aligned; `count` must match the allocation.
    pub fn free_frames(&mut self, phys: usize, count: usize) {
        assert!(phys % PAGE_SIZE == 0, "phys {phys:#x} not page-aligned");
        let ppn = phys >> 12;
        let freed = count.next_power_of_two();
        self.inner.dealloc(ppn, count);
        self.free_frames += freed;
    }

    /// Number of free 4 KiB frames tracked by the allocator.
    pub fn available_frames(&self) -> usize {
        self.free_frames
    }
}

// ---------------------------------------------------------------------------
// Global allocator instance
// ---------------------------------------------------------------------------

pub(crate) struct Allocator(pub(crate) UnsafeCell<FrameAllocator>);

// SAFETY: Only initialised once (in `mm::init`) and accessed from a single core.
unsafe impl Sync for Allocator {}

pub static ALLOCATOR: Allocator = Allocator(UnsafeCell::new(FrameAllocator::new()));

/// Allocate `count` contiguous 4 KiB physical frames.
pub fn alloc_frames(count: usize) -> Option<usize> {
    unsafe { (*ALLOCATOR.0.get()).alloc_frames(count) }
}

/// Free `count` contiguous 4 KiB frames at `phys`.
pub fn free_frames(phys: usize, count: usize) {
    unsafe { (*ALLOCATOR.0.get()).free_frames(phys, count) }
}

/// Allocate one 4 KiB physical page.
pub fn alloc_page() -> Option<usize> {
    alloc_frames(1)
}

/// Allocate one 2 MiB physically contiguous huge page (512 × 4 KiB).
pub fn alloc_huge_2m() -> Option<usize> {
    alloc_frames(FRAMES_2M)
}

#[allow(dead_code)]
pub fn available_frames() -> usize {
    unsafe { (*ALLOCATOR.0.get()).available_frames() }
}
