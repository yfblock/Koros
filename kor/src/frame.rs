//! Physical page-frame allocator.
//!
//! A *frame* is a physical page — the physical-memory counterpart of the
//! virtual pages managed by page tables (`kor-mmu`). Registering free
//! regions (`add_region`) hands raw RAM to this module; `alloc_frame(s)`
//! hands it out as 2^n-aligned physical extents.
//!
//! The global instance is protected by an IRQ-safe raw spin lock so
//! allocator metadata cannot race on SMP and cannot recursively allocate
//! while that lock is held.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

/// Size of one physical page frame (4 KiB).
pub const PAGE_SIZE: usize = 4096;

/// Number of 4 KiB frames in a 2 MiB huge page.
pub const FRAMES_2M: usize = 512;

/// Maximum number of independently registered physical regions.
const MAX_REGIONS: usize = 64;
/// Metadata capacity. Current supported machines expose at most 1 GiB, while
/// this tracks up to 4 GiB of registered RAM without allocating heap memory.
pub const MAX_TRACKED_FRAMES: usize = 1 << 20;
const BITMAP_WORDS: usize = MAX_TRACKED_FRAMES / usize::BITS as usize;

#[derive(Clone, Copy)]
struct Region {
    start_ppn: usize,
    frames: usize,
    bitmap_base: usize,
    next_hint: usize,
}

impl Region {
    const EMPTY: Self = Self {
        start_ppn: 0,
        frames: 0,
        bitmap_base: 0,
        next_hint: 0,
    };
}

/// Fixed-metadata contiguous physical-frame allocator.
///
/// A set bit denotes an allocated frame. Region metadata and the bitmap live
/// inside the allocator, so alloc/free never invoke the kernel heap.
pub struct FrameAllocator {
    regions: [Region; MAX_REGIONS],
    region_count: usize,
    tracked_frames: usize,
    free_frames: usize,
    allocated: [usize; BITMAP_WORDS],
}

impl FrameAllocator {
    pub const fn new() -> Self {
        Self {
            regions: [Region::EMPTY; MAX_REGIONS],
            region_count: 0,
            tracked_frames: 0,
            free_frames: 0,
            allocated: [0; BITMAP_WORDS],
        }
    }

    /// Register free physical memory `[start, end)` (byte addresses).
    /// Non-page-aligned bounds are rounded inward.
    pub fn add_region(&mut self, start: usize, end: usize) {
        let start = start.saturating_add(PAGE_SIZE - 1) & !(PAGE_SIZE - 1); // align-up (pow2)
        let end = end & !(PAGE_SIZE - 1);
        if start >= end {
            return;
        }
        let start_ppn = start / PAGE_SIZE;
        let frames = (end - start) / PAGE_SIZE;
        assert!(
            self.region_count < MAX_REGIONS,
            "too many physical memory regions"
        );
        assert!(
            self.tracked_frames + frames <= MAX_TRACKED_FRAMES,
            "physical memory exceeds frame metadata capacity"
        );
        for region in &self.regions[..self.region_count] {
            let region_end = region.start_ppn + region.frames;
            assert!(
                start_ppn + frames <= region.start_ppn || start_ppn >= region_end,
                "overlapping physical memory regions"
            );
        }
        self.regions[self.region_count] = Region {
            start_ppn,
            frames,
            bitmap_base: self.tracked_frames,
            next_hint: 0,
        };
        self.region_count += 1;
        self.tracked_frames += frames;
        self.free_frames += frames;
    }

    /// Allocate `count` contiguous 4 KiB frames. The reserved extent is rounded
    /// up to a power of two and aligned to that same number of frames.
    pub fn alloc_frames(&mut self, count: usize) -> Option<usize> {
        if count == 0 {
            return None;
        }
        let frames = count.checked_next_power_of_two()?;
        for region_index in 0..self.region_count {
            if frames > self.regions[region_index].frames {
                continue;
            }
            let hint = self.regions[region_index].next_hint;
            if let Some(offset) = self
                .find_free_run(
                    region_index,
                    hint,
                    self.regions[region_index].frames,
                    frames,
                )
                .or_else(|| self.find_free_run(region_index, 0, hint, frames))
            {
                self.mark_run(region_index, offset, frames, true);
                self.regions[region_index].next_hint = offset + frames;
                self.free_frames -= frames;
                return Some((self.regions[region_index].start_ppn + offset) * PAGE_SIZE);
            }
        }
        None
    }

    /// Free `count` contiguous frames previously returned by [`alloc_frames`].
    fn free_frames_checked(
        &mut self,
        phys: usize,
        count: usize,
        caller: &'static core::panic::Location<'static>,
    ) {
        assert!(count != 0, "cannot free zero frames");
        assert!(phys % PAGE_SIZE == 0, "phys {phys:#x} not page-aligned");
        let frames = count
            .checked_next_power_of_two()
            .expect("frame count overflow");
        let ppn = phys / PAGE_SIZE;
        let (region_index, offset) = self
            .locate(ppn, frames)
            .unwrap_or_else(|| panic!("frame extent not registered: {phys:#x} x{count}"));
        for index in offset..offset + frames {
            assert!(
                self.bit(region_index, index),
                "freeing unallocated frame {phys:#x} x{count} from {caller}"
            );
        }
        self.mark_run(region_index, offset, frames, false);
        self.regions[region_index].next_hint = self.regions[region_index].next_hint.min(offset);
        self.free_frames += frames;
    }

    /// Number of free 4 KiB frames tracked by the allocator.
    pub fn available_frames(&self) -> usize {
        self.free_frames
    }

    fn locate(&self, ppn: usize, frames: usize) -> Option<(usize, usize)> {
        self.regions[..self.region_count]
            .iter()
            .enumerate()
            .find_map(|(index, region)| {
                let offset = ppn.checked_sub(region.start_ppn)?;
                (offset.checked_add(frames)? <= region.frames).then_some((index, offset))
            })
    }

    fn find_free_run(
        &self,
        region_index: usize,
        start: usize,
        end: usize,
        frames: usize,
    ) -> Option<usize> {
        let region = self.regions[region_index];
        let mut offset =
            align_ppn(region.start_ppn + start, frames).checked_sub(region.start_ppn)?;
        while offset.checked_add(frames)? <= end {
            let mut first_used = None;
            for index in offset..offset + frames {
                if self.bit(region_index, index) {
                    first_used = Some(index);
                    break;
                }
            }
            if let Some(used) = first_used {
                offset =
                    align_ppn(region.start_ppn + used + 1, frames).checked_sub(region.start_ppn)?;
            } else {
                return Some(offset);
            }
        }
        None
    }

    fn bit(&self, region_index: usize, offset: usize) -> bool {
        let bit = self.regions[region_index].bitmap_base + offset;
        self.allocated[bit / usize::BITS as usize] & (1usize << (bit % usize::BITS as usize)) != 0
    }

    fn mark_run(&mut self, region_index: usize, offset: usize, frames: usize, value: bool) {
        let base = self.regions[region_index].bitmap_base;
        for bit in base + offset..base + offset + frames {
            let word = &mut self.allocated[bit / usize::BITS as usize];
            let mask = 1usize << (bit % usize::BITS as usize);
            if value {
                *word |= mask;
            } else {
                *word &= !mask;
            }
        }
    }
}

const fn align_ppn(ppn: usize, frames: usize) -> usize {
    (ppn + frames - 1) & !(frames - 1)
}

struct GlobalAllocator {
    lock: AtomicBool,
    inner: UnsafeCell<FrameAllocator>,
}

// SAFETY: all access to `inner` is serialized by `lock` with local interrupts
// disabled, including interrupt-context allocation on the same CPU.
unsafe impl Sync for GlobalAllocator {}

static ALLOCATOR: GlobalAllocator = GlobalAllocator {
    lock: AtomicBool::new(false),
    inner: UnsafeCell::new(FrameAllocator::new()),
};

fn with_allocator<R>(operation: impl FnOnce(&mut FrameAllocator) -> R) -> R {
    while ALLOCATOR
        .lock
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    // SAFETY: the global raw lock is held, serializing all alloc/free.
    let result = operation(unsafe { &mut *ALLOCATOR.inner.get() });
    ALLOCATOR.lock.store(false, Ordering::Release);
    result
}

/// Register free physical memory `[start, end)` with the global allocator.
pub fn add_region(start: usize, end: usize) {
    with_allocator(|allocator| allocator.add_region(start, end));
}

/// Allocate `count` contiguous 4 KiB physical frames.
pub fn alloc_frames(count: usize) -> Option<usize> {
    with_allocator(|allocator| allocator.alloc_frames(count))
}

/// Free `count` contiguous 4 KiB frames at `phys`.
#[track_caller]
pub fn free_frames(phys: usize, count: usize) {
    let caller = core::panic::Location::caller();
    with_allocator(|allocator| allocator.free_frames_checked(phys, count, caller));
}

/// Allocate one 4 KiB physical frame.
pub fn alloc_frame() -> Option<usize> {
    alloc_frames(1)
}

/// Allocate one 2 MiB physically contiguous huge page.
pub fn alloc_huge_2m() -> Option<usize> {
    alloc_frames(FRAMES_2M)
}

/// Number of free 4 KiB frames.
pub fn available_frames() -> usize {
    with_allocator(|allocator| allocator.available_frames())
}
