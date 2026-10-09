//! Physical / virtual address newtypes, the installed phys↔virt translation,
//! and typed page-table accessors.
//!
//! The translation is an *application* property (a privilege-level / link-
//! model fact): koros runs unmapped (identity), korix uses the high-half
//! window, korhv is identity at EL2 on aarch64 and high-half on x86_64.  Each
//! application installs its pair once at boot via [`install_translation`];
//! until then the identity is used.  All crates route frame and device
//! address translation through these newtypes, so PA/VA confusion is a type
//! error instead of a runtime fault.

use core::sync::atomic::{AtomicUsize, Ordering};

type TranslateFn = fn(usize) -> usize;

/// Installed phys→virt translation (0 = not yet installed → identity).
static PHYS_TO_VIRT: AtomicUsize = AtomicUsize::new(0);
/// Installed virt→phys translation (0 = not yet installed → identity).
static VIRT_TO_PHYS: AtomicUsize = AtomicUsize::new(0);

/// Install the application's phys↔virt translation.  Call once, before any
/// frame-backed allocation or device probing.  Deliberately allocation-free:
/// it must be usable before the heap exists.
pub fn install_translation(phys_to_virt: TranslateFn, virt_to_phys: TranslateFn) {
    PHYS_TO_VIRT.store(phys_to_virt as usize, Ordering::Release);
    VIRT_TO_PHYS.store(virt_to_phys as usize, Ordering::Release);
}

fn translate_up(pa: usize) -> usize {
    let f = PHYS_TO_VIRT.load(Ordering::Acquire);
    if f == 0 {
        pa
    } else {
        // SAFETY: the value was stored from a valid fn pointer above.
        (unsafe { core::mem::transmute::<usize, TranslateFn>(f) })(pa)
    }
}

fn translate_down(va: usize) -> usize {
    let f = VIRT_TO_PHYS.load(Ordering::Acquire);
    if f == 0 {
        va
    } else {
        // SAFETY: the value was stored from a valid fn pointer above.
        (unsafe { core::mem::transmute::<usize, TranslateFn>(f) })(va)
    }
}

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

    /// Translate to the kernel virtual address through the installed
    /// application translation (identity until installed).
    #[inline]
    pub fn to_va(self) -> VirtAddr {
        VirtAddr(translate_up(self.0))
    }

    /// Volatile read of a `T`-wide device register at this address,
    /// translated through the installed application pair.
    #[inline]
    pub fn read<T: Copy>(&self) -> T {
        self.to_va().read::<T>()
    }

    /// Volatile write of a `T`-wide device register at this address,
    /// translated through the installed application pair.
    #[inline]
    pub fn write<T: Copy>(&self, value: T) {
        self.to_va().write::<T>(value)
    }

    /// View this page through the installed translation as a mutable slice.
    ///
    /// # Safety
    /// The page must be valid for `len` elements of `T` and exclusively
    /// owned for the returned lifetime (no aliases while it is live).
    #[inline]
    pub unsafe fn page_slice_at_mut<T>(self, len: usize) -> &'static mut [T] {
        let va = self.to_va().raw();
        // SAFETY: caller guarantees validity and exclusivity.
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

    /// Translate back to the physical address through the installed
    /// application translation (identity until installed).
    #[inline]
    pub fn to_pa(self) -> PhysAddr {
        PhysAddr(translate_down(self.0))
    }

    /// Volatile read of a `T`-wide cell at this address.
    ///
    /// The address must name a valid mapped cell of width `T` (a device
    /// register from the platform constants, or owned memory).
    #[inline]
    pub fn read<T: Copy>(&self) -> T {
        // SAFETY: caller guarantees a valid, mapped, non-aliased-for-read cell.
        unsafe { (self.0 as *const T).read_volatile() }
    }

    /// Volatile write of a `T`-wide cell at this address.
    ///
    /// The address must name a valid mapped cell of width `T`.
    #[inline]
    pub fn write<T: Copy>(&self, value: T) {
        // SAFETY: caller guarantees a valid, mapped cell.
        unsafe { (self.0 as *mut T).write_volatile(value) }
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
