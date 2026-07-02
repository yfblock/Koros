//! Slab-based kernel heap — fixed-size caches backed by buddy frames on demand.
//!
//! Objects up to 4 KiB use power-of-two size classes (64 … 4096 bytes).
//! Larger allocations come directly from the physical frame buddy allocator.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::NonNull;

use spin::Mutex;

use crate::mm::frame_allocator::{self, PAGE_SIZE};
use crate::mm::{phys_to_virt, virt_to_phys};

const NUM_SLABS: usize = 7;
const MIN_SLAB_BYTES: usize = PAGE_SIZE;
/// Minimum contiguous region for [`SlabHeap::new`] (bootstrap only).
pub const MIN_HEAP_BYTES: usize = NUM_SLABS * MIN_SLAB_BYTES;

/// Pages added to one size class when it runs dry.
const GROW_PAGES: usize = 8;

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
    const fn empty(block_size: usize) -> Self {
        Self {
            block_size,
            free_list: FreeList::empty(),
        }
    }

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
pub struct SlabHeap {
    slab_64: Slab,
    slab_128: Slab,
    slab_256: Slab,
    slab_512: Slab,
    slab_1024: Slab,
    slab_2048: Slab,
    slab_4096: Slab,
    frame_bytes: usize,
}

impl SlabHeap {
    /// Bootstrap layout: split `[start, start + size)` evenly across size classes.
    ///
    /// # Safety
    /// `[start, start + size)` must be unused, page-aligned at `start`, and valid for `'static`.
    pub unsafe fn new(start: usize, size: usize) -> Self {
        assert!(start % MIN_SLAB_BYTES == 0, "heap start {start:#x} not page-aligned");
        assert!(size >= MIN_HEAP_BYTES, "heap size {size:#x} below minimum {MIN_HEAP_BYTES:#x}");
        assert!(
            size % MIN_HEAP_BYTES == 0,
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
                frame_bytes: 0,
            }
        }
    }

    fn class_for(layout: &Layout) -> SlabClass {
        if layout.size() > 4096 {
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
            self.frame_bytes += bytes;
        }
    }

    fn allocate(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        match Self::class_for(&layout) {
            SlabClass::Large => Self::alloc_large(layout),
            class => self.slab_mut(class)?.allocate(),
        }
    }

    fn alloc_large(layout: Layout) -> Option<NonNull<u8>> {
        let pages = layout.size().div_ceil(PAGE_SIZE).max(1);
        let phys = frame_allocator::alloc_frames(pages)?;
        let va = phys_to_virt(phys);
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
        let pages = layout.size().div_ceil(PAGE_SIZE).max(1);
        let phys = virt_to_phys(ptr.as_ptr() as usize);
        frame_allocator::free_frames(phys, pages);
    }

    pub fn frame_backed_bytes(&self) -> usize {
        self.frame_bytes
    }
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

    fn grow_from_frames(&self, class: SlabClass) -> bool {
        if class == SlabClass::Large {
            return false;
        }
        let Some(phys) = frame_allocator::alloc_frames(GROW_PAGES) else {
            return false;
        };
        let start = phys_to_virt(phys);
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

        // Large allocations go directly to the frame allocator, bypassing the
        // slab mutex.  This is critical because `alloc_frames` may trigger
        // re-entrant allocations via the buddy allocator's internal BTreeSet
        // operations — holding the slab spin-lock during that would deadlock.
        if class == SlabClass::Large {
            let pages = layout.size().div_ceil(PAGE_SIZE).max(1);
            if let Some(phys) = frame_allocator::alloc_frames(pages) {
                return phys_to_virt(phys) as *mut u8;
            }
            return core::ptr::null_mut();
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
                // Large deallocation — same reasoning as alloc: avoid holding
                // the slab lock while calling into the frame allocator.
                let pages = layout.size().div_ceil(PAGE_SIZE).max(1);
                let phys = virt_to_phys(ptr as usize);
                frame_allocator::free_frames(phys, pages);
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
