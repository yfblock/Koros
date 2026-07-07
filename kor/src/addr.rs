//! Physical / virtual Physical / virtual address helpers (used by page-table code in `kor-arch`).

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

    #[inline]
    pub fn page_slice_mut<T>(self, len: usize) -> &'static mut [T] {
        let va = crate::arch::phys_to_virt(self.0);
        unsafe { core::slice::from_raw_parts_mut(va as *mut T, len) }
    }

    #[inline]
    pub fn clear_page(self) {
        self.page_slice_mut::<u8>(4096).fill(0);
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
