//! Per page-table-page descriptors (CortenMM's single-level abstraction).
//!
//! Every page-table page is associated with a [`PageDesc`], indexed by the
//! page frame number of the table page.  The descriptor owns
//! 1. the lock that serializes transactions touching the table page, and
//! 2. the per-PTE metadata array (allocated on demand, freed with the
//!    page) that stores on-demand-paging / copy-on-write bookkeeping
//!    (see [`PageStatus`]).
//!
//! The descriptor table itself is a fixed-size array covering the frame
//! range tracked by `kor-frame`, so lookups never allocate.

use alloc::boxed::Box;

use core::sync::atomic::{AtomicUsize, Ordering};

use spin::Mutex;

use kor::mapping::MappingFlags;

/// Entries per page-table page (64-bit PTEs, 4 KiB pages).
pub const PT_ENTRIES: usize = 512;

/// State of one virtual page, mirroring the hardware PTE plus the extra
/// semantics state CortenMM keeps beside the page table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageStatus {
    /// Nothing is known about the page.
    Invalid,
    /// The hardware PTE maps the page (physical address, permission, size).
    Mapped {
        phys: usize,
        flags: MappingFlags,
        /// Leaf granularity in bytes (4096, 2 MiB, or 1 GiB).
        granularity: usize,
    },
    /// Virtually allocated (e.g. by an anonymous `mmap`): the permission is
    /// remembered, the mapping is created later on page fault.
    Allocated(MappingFlags),
}

/// Per-PTE metadata array, allocated on demand from the kernel heap.
pub type PteMetadata = [PageStatus; PT_ENTRIES];

/// Descriptor of one page-table page.
pub struct PageDesc {
    /// Serializes transactions operating through this table page.
    lock: Mutex<()>,
    /// Per-PTE metadata pointer (`0` = not allocated); allocated lazily,
    /// dropped when the table page is freed.  Guarded by `lock`.
    metadata: AtomicUsize,
}

impl PageDesc {
    const fn new() -> Self {
        Self {
            lock: Mutex::new(()),
            metadata: AtomicUsize::new(0),
        }
    }

    /// Lock the table page (part of a transaction's lock set) and return
    /// the RAII guard; dropping it releases the page.
    ///
    /// Descriptors are `&'static`, so the guard outlives any cursor that
    /// stores it.
    pub fn acquire(&self) -> spin::MutexGuard<'_, ()> {
        self.lock.lock()
    }

    /// Read the per-PTE status at `index`, if metadata exists.
    ///
    /// # Safety
    /// The caller holds this descriptor's lock (so the array cannot be
    /// concurrently freed) — except during page teardown, where the page is
    /// already unreachable.
    pub unsafe fn status(&self, index: usize) -> PageStatus {
        let ptr = self.metadata.load(Ordering::Relaxed);
        if ptr == 0 {
            return PageStatus::Invalid;
        }
        // SAFETY: see the safety contract above.
        unsafe { (*(ptr as *const PteMetadata))[index] }
    }

    /// Write the per-PTE status at `index`, allocating the array on demand.
    ///
    /// # Safety
    /// The caller holds this descriptor's lock.
    pub unsafe fn set_status(&self, index: usize, status: PageStatus) {
        let mut ptr = self.metadata.load(Ordering::Relaxed);
        if ptr == 0 {
            let boxed: Box<PteMetadata> = Box::new([PageStatus::Invalid; PT_ENTRIES]);
            ptr = Box::into_raw(boxed) as usize;
            self.metadata.store(ptr, Ordering::Relaxed);
        }
        // SAFETY: caller holds the lock; the array is valid while locked.
        unsafe { (*(ptr as *mut PteMetadata))[index] = status };
    }

    /// Drop the metadata array (called when the table page is freed).
    ///
    /// # Safety
    /// The table page must be unreachable from any page table.
    pub unsafe fn drop_metadata(&self) {
        let ptr = self.metadata.swap(0, Ordering::Relaxed);
        if ptr != 0 {
            // SAFETY: the pointer came from `Box::into_raw` and the page is
            // unreachable, so nobody else can observe it.
            unsafe { drop(Box::from_raw(ptr as *mut PteMetadata)) };
        }
    }
}

/// Number of descriptors: matches `kor-frame`'s tracked-frame capacity.
const MAX_DESCRIPTORS: usize = kor::frame::MAX_TRACKED_FRAMES;

/// Global descriptor table, indexed by frame number.  `PageDesc` uses
/// interior mutability, so the array itself is immutable.
static DESCRIPTORS: [PageDesc; MAX_DESCRIPTORS] = [const { PageDesc::new() }; MAX_DESCRIPTORS];

/// Descriptor of the page-table page living in `frame` (physical address).
///
/// # Panics
/// Panics if `frame` is not page aligned or beyond the tracked range.
pub fn descriptor_of(frame: usize) -> &'static PageDesc {
    assert!(frame % kor::frame::PAGE_SIZE == 0, "unaligned table frame");
    let index = frame / kor::frame::PAGE_SIZE;
    assert!(index < MAX_DESCRIPTORS, "frame beyond descriptor table");
    &DESCRIPTORS[index]
}
