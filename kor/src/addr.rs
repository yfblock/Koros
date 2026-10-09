//! Physical / virtual address helpers (used by page-table code).

/// Physical address (can be used before/without a direct map).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysAddr(usize);

impl PhysAddr {
    #[inline]
    pub const fn new(addr: usize) -> Self {
        Self(addr)
    }

    #[inline]
    pub const fn raw(self) -> usize {
        self.0
    }

    /// View this page at an already-translated virtual address.
    ///
    /// # Safety
    /// `va` must be the correct kernel VA for this physical page, the page
    /// must be valid for `len` elements of `T`, and access must be
    /// serialized (no aliases while the slice is live).
    #[inline]
    pub unsafe fn page_slice_at_mut<T>(self, va: usize, len: usize) -> &'static mut [T] {
        // SAFETY: caller guarantees the VA translation and exclusivity.
        unsafe { core::slice::from_raw_parts_mut(va as *mut T, len) }
    }
}

/// Virtual address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct VirtAddr(usize);

impl VirtAddr {
    #[inline]
    pub const fn new(addr: usize) -> Self {
        Self(addr)
    }

    #[inline]
    pub const fn raw(self) -> usize {
        self.0
    }

    /// Level `n` page-table index (n = 0 is the 4 KiB level).
    #[inline]
    pub const fn pn_index(self, n: usize) -> usize {
        (self.0 >> (12 + 9 * n)) & 0x1ff
    }

    /// Byte offset within the mapping at level `n`.
    #[inline]
    pub const fn page_offset(self, n: usize) -> usize {
        self.0 % (1 << (12 + 9 * n))
    }
}
