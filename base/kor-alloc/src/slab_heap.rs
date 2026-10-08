//! Slab-based kernel heap — fixed-size caches backed by buddy frames on demand.
//!
//! Objects up to 4 KiB use power-of-two size classes (64 … 4096 bytes).
//! Larger allocations come directly from the physical frame buddy allocator.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::NonNull;

use spin::Mutex;

use kor::frame::PAGE_SIZE;

// Address translation (phys↔virt) for frame-backed memory comes from the
// installed `kor` architecture provider.

const NUM_SLABS: usize = 7;
const MIN_SLAB_BYTES: usize = PAGE_SIZE;
/// Minimum contiguous region for [`SlabHeap::new`] (bootstrap only).
pub const MIN_HEAP_BYTES: usize = NUM_SLABS * MIN_SLAB_BYTES;

/// Pages added to one size class when it runs dry.
const GROW_PAGES: usize = 8;
/// Whether frame-backed allocation is available after physical-memory discovery.
static FRAME_GROWTH_ENABLED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlabClass {
    Bytes64,
    Bytes128,
    Bytes256,
    Bytes512,
    Bytes1024,
    Bytes2048,
    Bytes4096,
    Large,
}

/// One size-class slab backed by a free-object list.
struct Slab {
    block_size: usize,
    free_list: FreeList,
}

impl Slab {
    unsafe fn new(start: usize, bytes: usize, block_size: usize) -> Self {
        let count = bytes / block_size;
        Self {
            block_size,
            free_list: FreeList::new(start, block_size, count),
        }
    }

    unsafe fn grow(&mut self, start: usize, bytes: usize) {
        let count = bytes / self.block_size;
        let mut incoming = FreeList::new(start, self.block_size, count);
        while let Some(addr) = incoming.pop() {
            self.free_list.push(addr);
        }
    }

    fn allocate(&mut self) -> Option<NonNull<u8>> {
        self.free_list
            .pop()
            .map(|addr| unsafe { NonNull::new_unchecked(addr as *mut u8) })
    }

    unsafe fn deallocate(&mut self, ptr: NonNull<u8>) {
        self.free_list.push(ptr.as_ptr() as usize);
    }
}

struct FreeList {
    head: *mut FreeNode,
}

unsafe impl Send for FreeList {}

struct FreeNode {
    next: *mut FreeNode,
}

impl FreeList {
    const fn empty() -> Self {
        Self {
            head: core::ptr::null_mut(),
        }
    }

    fn new(start: usize, block_size: usize, count: usize) -> Self {
        let mut list = Self::empty();
        for i in (0..count).rev() {
            list.push(start + i * block_size);
        }
        list
    }

    fn pop(&mut self) -> Option<usize> {
        if self.head.is_null() {
            return None;
        }
        unsafe {
            let node = self.head;
            self.head = (*node).next;
            Some(node as usize)
        }
    }

    fn push(&mut self, addr: usize) {
        let node = addr as *mut FreeNode;
        unsafe {
            (*node).next = self.head;
            self.head = node;
        }
    }
}

/// Multi-slab heap with seven object caches; large blocks use frames directly.
struct SlabHeap {
    slab_64: Slab,
    slab_128: Slab,
    slab_256: Slab,
    slab_512: Slab,
    slab_1024: Slab,
    slab_2048: Slab,
    slab_4096: Slab,
}

impl SlabHeap {
    /// Bootstrap layout: split `[start, start + size)` evenly across size classes.
    ///
    /// # Safety
    /// `[start, start + size)` must be unused, page-aligned at `start`, and valid for `'static`.
    pub unsafe fn new(start: usize, size: usize) -> Self {
        assert!(
            start.is_multiple_of(MIN_SLAB_BYTES),
            "heap start {start:#x} not page-aligned"
        );
        assert!(
            size >= MIN_HEAP_BYTES,
            "heap size {size:#x} below minimum {MIN_HEAP_BYTES:#x}"
        );
        assert!(
            size.is_multiple_of(MIN_HEAP_BYTES),
            "heap size must be a multiple of {MIN_HEAP_BYTES:#x}"
        );

        let slot = size / NUM_SLABS;
        unsafe {
            Self {
                slab_64: Slab::new(start, slot, 64),
                slab_128: Slab::new(start + slot, slot, 128),
                slab_256: Slab::new(start + 2 * slot, slot, 256),
                slab_512: Slab::new(start + 3 * slot, slot, 512),
                slab_1024: Slab::new(start + 4 * slot, slot, 1024),
                slab_2048: Slab::new(start + 5 * slot, slot, 2048),
                slab_4096: Slab::new(start + 6 * slot, slot, 4096),
            }
        }
    }

    fn class_for(layout: &Layout) -> SlabClass {
        if layout.size() > PAGE_SIZE || layout.align() > PAGE_SIZE {
            SlabClass::Large
        } else if layout.size() <= 64 && layout.align() <= 64 {
            SlabClass::Bytes64
        } else if layout.size() <= 128 && layout.align() <= 128 {
            SlabClass::Bytes128
        } else if layout.size() <= 256 && layout.align() <= 256 {
            SlabClass::Bytes256
        } else if layout.size() <= 512 && layout.align() <= 512 {
            SlabClass::Bytes512
        } else if layout.size() <= 1024 && layout.align() <= 1024 {
            SlabClass::Bytes1024
        } else if layout.size() <= 2048 && layout.align() <= 2048 {
            SlabClass::Bytes2048
        } else {
            SlabClass::Bytes4096
        }
    }

    fn slab_mut(&mut self, class: SlabClass) -> Option<&mut Slab> {
        match class {
            SlabClass::Bytes64 => Some(&mut self.slab_64),
            SlabClass::Bytes128 => Some(&mut self.slab_128),
            SlabClass::Bytes256 => Some(&mut self.slab_256),
            SlabClass::Bytes512 => Some(&mut self.slab_512),
            SlabClass::Bytes1024 => Some(&mut self.slab_1024),
            SlabClass::Bytes2048 => Some(&mut self.slab_2048),
            SlabClass::Bytes4096 => Some(&mut self.slab_4096),
            SlabClass::Large => None,
        }
    }

    unsafe fn grow(&mut self, class: SlabClass, start: usize, bytes: usize) {
        if let Some(slab) = self.slab_mut(class) {
            unsafe {
                slab.grow(start, bytes);
            }
        }
    }

    fn allocate(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        match Self::class_for(&layout) {
            SlabClass::Large => Self::alloc_large(layout),
            class => self.slab_mut(class)?.allocate(),
        }
    }

    fn alloc_large(layout: Layout) -> Option<NonNull<u8>> {
        if !FRAME_GROWTH_ENABLED.load(core::sync::atomic::Ordering::Acquire) {
            return None;
        }
        let frames = large_extent_frames(layout)?;
        let phys = kor::frame::alloc_frames(frames)?;
        let va = kor::arch::phys_to_virt(phys);
        debug_assert_eq!(va & (layout.align() - 1), 0);
        Some(unsafe { NonNull::new_unchecked(va as *mut u8) })
    }

    unsafe fn deallocate(&mut self, ptr: NonNull<u8>, layout: Layout) {
        match Self::class_for(&layout) {
            SlabClass::Large => Self::dealloc_large(ptr, layout),
            class => {
                if let Some(slab) = self.slab_mut(class) {
                    unsafe {
                        slab.deallocate(ptr);
                    }
                }
            }
        }
    }

    fn dealloc_large(ptr: NonNull<u8>, layout: Layout) {
        let frames = large_extent_frames(layout).expect("valid allocation layout");
        let phys = kor::arch::virt_to_phys(ptr.as_ptr() as usize);
        kor::frame::free_frames(phys, frames);
    }
}

fn large_extent_frames(layout: Layout) -> Option<usize> {
    let size_frames = layout.size().max(1).checked_next_multiple_of(PAGE_SIZE)? / PAGE_SIZE;
    let align_frames = layout.align().max(PAGE_SIZE) / PAGE_SIZE;
    Some(size_frames.max(align_frames))
}

/// Spin-locked slab heap suitable for `#[global_allocator]`.
pub struct LockedSlabHeap(Mutex<Option<SlabHeap>>);

impl LockedSlabHeap {
    pub const fn empty() -> Self {
        Self(Mutex::new(None))
    }

    /// # Safety
    /// Same requirements as [`SlabHeap::new`].
    pub unsafe fn init(&self, start: usize, size: usize) {
        *self.0.lock() = Some(unsafe { SlabHeap::new(start, size) });
    }

    /// Allow slab exhaustion and large allocations to draw pages from the
    /// physical-frame allocator. Call once after all RAM regions are registered.
    pub fn enable_frame_growth(&self) {
        FRAME_GROWTH_ENABLED.store(true, core::sync::atomic::Ordering::Release);
    }

    fn grow_from_frames(&self, class: SlabClass) -> bool {
        if !FRAME_GROWTH_ENABLED.load(core::sync::atomic::Ordering::Acquire) {
            return false;
        }
        if class == SlabClass::Large {
            return false;
        }
        let Some(phys) = kor::frame::alloc_frames(GROW_PAGES) else {
            return false;
        };
        let start = kor::arch::phys_to_virt(phys);
        let bytes = GROW_PAGES * PAGE_SIZE;
        let mut guard = self.0.lock();
        if let Some(heap) = guard.as_mut() {
            unsafe {
                heap.grow(class, start, bytes);
            }
            true
        } else {
            false
        }
    }
}

unsafe impl GlobalAlloc for LockedSlabHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let class = SlabHeap::class_for(&layout);

        // Large or over-aligned allocations draw a right-sized contiguous
        // extent directly from physical memory. The single Large path lives
        // in `SlabHeap::alloc_large`, gated on frame growth.
        if class == SlabClass::Large {
            return match SlabHeap::alloc_large(layout) {
                Some(ptr) => ptr.as_ptr(),
                None => core::ptr::null_mut(),
            };
        }

        loop {
            {
                let mut guard = self.0.lock();
                if let Some(heap) = guard.as_mut() {
                    if let Some(ptr) = heap.allocate(layout) {
                        return ptr.as_ptr();
                    }
                } else {
                    return core::ptr::null_mut();
                }
            }
            if !self.grow_from_frames(class) {
                return core::ptr::null_mut();
            }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(nn) = NonNull::new(ptr) {
            let class = SlabHeap::class_for(&layout);
            if class == SlabClass::Large {
                // Use the identical extent calculation as allocation so the
                // frame allocator releases the full aligned reservation.
                let frames = large_extent_frames(layout).expect("valid allocation layout");
                let phys = kor::arch::virt_to_phys(ptr as usize);
                kor::frame::free_frames(phys, frames);
            } else {
                let mut guard = self.0.lock();
                if let Some(heap) = guard.as_mut() {
                    unsafe {
                        heap.deallocate(nn, layout);
                    }
                }
            }
        }
    }
}
