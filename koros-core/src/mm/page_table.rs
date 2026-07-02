//! Virtual memory — dynamic page mapping on top of the boot page tables.

pub use crate::mm::addr::{PhysAddr, VirtAddr};

use crate::mm::frame_allocator::{self, FRAMES_2M, PAGE_SIZE};

#[cfg(target_arch = "riscv64")]
use crate::arch::riscv64::page_table as arch_page_table;
#[cfg(target_arch = "x86_64")]
use crate::arch::x86_64::page_table as arch_page_table;
#[cfg(target_arch = "aarch64")]
use crate::arch::aarch64::page_table as arch_page_table;
#[cfg(target_arch = "loongarch64")]
use crate::arch::loongarch64::page_table as arch_page_table;

/// Mapping permissions for [`map`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappingFlags(u8);

impl MappingFlags {
    pub const R: Self = Self(1 << 0);
    pub const W: Self = Self(1 << 1);
    pub const X: Self = Self(1 << 2);
    pub const U: Self = Self(1 << 3);

    pub const KERNEL_RWX: Self = Self(Self::R.0 | Self::W.0 | Self::X.0);

    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

/// Page size for a mapping operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapSize {
    Page4K,
    Page2M,
}

/// Errors from [`map`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    OutOfMemory,
    BlockedByExistingMapping,
    Unsupported,
}

/// Capture the active page-table root after boot.
pub fn init() {
    arch_page_table::init();
}

/// Map `paddr` at `vaddr` with the given size and flags.
pub fn map(vaddr: usize, paddr: usize, flags: MappingFlags, size: MapSize) -> Result<(), MapError> {
    arch_page_table::map(vaddr, paddr, flags, size)
}

/// Resolve `vaddr` to a physical address if mapped.
pub fn translate(vaddr: usize) -> Option<usize> {
    arch_page_table::translate(vaddr)
}

/// Whether this architecture supports runtime [`map`] in the current boot setup.
pub fn dynamic_maps_supported() -> bool {
    arch_page_table::dynamic_maps_supported()
}

/// Exercise frame allocation + page mapping.
pub fn self_test() {
    if !dynamic_maps_supported() {
        crate::println!("mm: page-table self-test skipped (no dynamic maps)");
        return;
    }

    const MARK: u64 = 0xDEAD_BEEF_CAFE_BABE;

    let phys = frame_allocator::alloc_page().expect("alloc_page");
    let va = arch_page_table::TEST_VA_4K;
    map(va, phys, MappingFlags::KERNEL_RWX, MapSize::Page4K).expect("map 4K");
    unsafe {
        (va as *mut u64).write_volatile(MARK);
        assert!((va as *const u64).read_volatile() == MARK);
    }
    assert_eq!(translate(va), Some(phys));

    let phys2m = frame_allocator::alloc_huge_2m().expect("alloc_huge_2m");
    // Sv39 L1 megapages take PA[20:12] from VA[20:12]; match those bits to the frame.
    let va2m = (arch_page_table::TEST_VA_2M & !0x1FF_000) | (phys2m & 0x1FF_000);
    map(va2m, phys2m, MappingFlags::KERNEL_RWX, MapSize::Page2M).expect("map 2M");
    assert_eq!(
        translate(va2m),
        Some(phys2m),
        "translate before write"
    );
    unsafe {
        (va2m as *mut u64).write_volatile(MARK);
        assert!((va2m as *const u64).read_volatile() == MARK);
    }
    assert_eq!(translate(va2m), Some(phys2m));
    crate::println!(
        "mm: page-table OK (4K {:#x}, 2M {:#x})",
        va,
        va2m
    );
    let _ = phys2m;

    let _ = (phys, PAGE_SIZE, FRAMES_2M);
}
