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

    /// Translate into the kernel's own mapping window (as [`KernelAddr`]) through the installed
    /// application translation (identity until installed).  The result is a
    /// [`KernelAddr`]: by construction it lies in memory the kernel itself
    /// has mapped, so it carries the volatile access rights.
    #[inline]
    pub fn to_ka(self) -> KernelAddr {
        KernelAddr::new(translate_up(self.0))
    }

    /// The physical address corresponding to `ka` under the installed
    /// application translation (identity until installed) — the reverse of
    /// [`to_ka`].  Owned by the physical side: neither `VirtAddr` nor
    /// `KernelAddr` carries the reverse translation as a method.
    #[inline]
    pub fn from_ka(ka: KernelAddr) -> Self {
        PhysAddr(translate_down(ka.0))
    }

    /// View this page through the installed translation as a mutable slice.
    ///
    /// # Safety
    /// The page must be valid for `len` elements of `T` and exclusively
    /// owned for the returned lifetime (no aliases while it is live).
    #[inline]
    pub unsafe fn page_slice_at_mut<T>(self, len: usize) -> &'static mut [T] {
        let va = self.to_ka().raw();
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

/// An address inside the kernel's own mapping window (the direct map /
/// high-half window of the running link model — identity for unmapped
/// kernels).  Only constructible through typed paths ([`PhysAddr::to_ka`]
/// and friends), which is what licenses the volatile cell access.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct KernelAddr(usize);

impl KernelAddr {
    /// Wrap an address already known to lie in the kernel's mapping window
    /// (e.g. a kernel-heap pointer).  Device and frame access should prefer
    /// the typed path, `PhysAddr::to_ka()`.
    #[inline]
    pub const fn new(addr: usize) -> Self {
        Self(addr)
    }

    #[inline]
    pub const fn raw(self) -> usize {
        self.0
    }

    /// Volatile read of a `T`-wide cell at this address.
    #[inline]
    pub fn read<T: Copy>(&self) -> T {
        // SAFETY: KernelAddr is only produced for kernel-mapped memory.
        unsafe { (self.0 as *const T).read_volatile() }
    }

    /// Volatile write of a `T`-wide cell at this address.
    #[inline]
    pub fn write<T: Copy>(&self, value: T) {
        // SAFETY: KernelAddr is only produced for kernel-mapped memory.
        unsafe { (self.0 as *mut T).write_volatile(value) }
    }

    /// Register cell at a fixed byte offset in the same window.
    #[inline]
    pub const fn offset(self, off: usize) -> Self {
        Self(self.0 + off)
    }

    /// Volatile `u8` read (device-register convenience).
    #[inline]
    pub fn read_u8(&self) -> u8 {
        self.read::<u8>()
    }

    /// Volatile `u16` read.
    #[inline]
    pub fn read_u16(&self) -> u16 {
        self.read::<u16>()
    }

    /// Volatile `u32` read.
    #[inline]
    pub fn read_u32(&self) -> u32 {
        self.read::<u32>()
    }

    /// Volatile `u64` read.
    #[inline]
    pub fn read_u64(&self) -> u64 {
        self.read::<u64>()
    }

    /// Volatile `u8` write.
    #[inline]
    pub fn write_u8(&self, value: u8) {
        self.write::<u8>(value)
    }

    /// Volatile `u16` write.
    #[inline]
    pub fn write_u16(&self, value: u16) {
        self.write::<u16>(value)
    }

    /// Volatile `u32` write.
    #[inline]
    pub fn write_u32(&self, value: u32) {
        self.write::<u32>(value)
    }

    /// Volatile `u64` write.
    #[inline]
    pub fn write_u64(&self, value: u64) {
        self.write::<u64>(value)
    }
}
