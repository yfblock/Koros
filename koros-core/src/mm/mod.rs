//! Physical memory management.
//!
//! Responsibilities:
//! - Detect available physical memory (per-arch code in `src/arch/<arch>/mm.rs`).
//! - Exclude the kernel image itself from the free pool.
//! - Initialise the [`frame_allocator`] (buddy system) so the rest of the kernel
//!   can allocate contiguous 4 KiB frames and 2 MiB huge pages.

mod frame_allocator;
mod heap;
mod page_table;
mod regions;
mod slab_heap;

pub(crate) mod addr;

pub use frame_allocator::{alloc_frames, alloc_huge_2m, alloc_page, available_frames, free_frames, FRAMES_2M, PAGE_SIZE};
pub use page_table::{map, translate, MapError, MapSize, MappingFlags};

// Shared FDT parser — used by riscv64, aarch64, and loongarch64.
// Excluded on x86_64 which uses Multiboot instead.
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "loongarch64"))]
pub(crate) mod fdt;

// ---------------------------------------------------------------------------
// Kernel-image boundaries (linker symbols) — subtracted from memory regions so
// the allocator doesn't give out frames occupied by the kernel.
// ---------------------------------------------------------------------------

unsafe extern "C" {
    static _skernel: u8;
    static _end: u8;
}

fn kernel_phys_range() -> (usize, usize) {
    let ko = arch_mm::kernel_offset();
    unsafe {
        let start = &_skernel as *const u8 as usize - ko;
        let end = &_end as *const u8 as usize - ko;
        (start, end)
    }
}

// ---------------------------------------------------------------------------
// Thin dispatch to per-arch memory detection
// ---------------------------------------------------------------------------

#[cfg(target_arch = "riscv64")]
use crate::arch::riscv64::mm as arch_mm;
#[cfg(target_arch = "x86_64")]
use crate::arch::x86_64::mm as arch_mm;
#[cfg(target_arch = "aarch64")]
use crate::arch::aarch64::mm as arch_mm;
#[cfg(target_arch = "loongarch64")]
use crate::arch::loongarch64::mm as arch_mm;

/// Convert a physical address to the kernel direct-map virtual address.
pub fn phys_to_virt(pa: usize) -> usize {
    arch_mm::phys_to_virt(pa)
}

/// Inverse of [`phys_to_virt`] for addresses in the direct map.
pub fn virt_to_phys(va: usize) -> usize {
    arch_mm::virt_to_phys(va)
}

/// The raw kernel command line supplied by the bootloader (FDT
/// `/chosen/bootargs` or the Multiboot cmdline), if any.
pub fn boot_cmdline() -> Option<alloc::string::String> {
    arch_mm::boot_cmdline()
}

// ---------------------------------------------------------------------------
// Initialisation
// ---------------------------------------------------------------------------

/// Initialise physical-memory management.
///
/// Must be called once, early in `kernel_main`, before any allocation.
pub fn init() {
    // Bootstrap heap for the frame buddy's internal `BTreeSet`.
    heap::init_bootstrap();

    // Capture the boot command line now, while the pages the bootloader used
    // for it (e.g. the Multiboot cmdline placed just past the kernel image)
    // are still intact — before the frame allocator can hand them out.
    crate::cmdline::init();

    let mut collected = regions::RegionCollector::new();
    arch_mm::init(|start, end| collected.add(start, end));

    let (ks, ke) = kernel_phys_range();
    // Round the reserved range outward so partial tail pages stay reserved.
    let ks = ks & !(frame_allocator::PAGE_SIZE - 1);
    let ke = (ke + frame_allocator::PAGE_SIZE - 1) & !(frame_allocator::PAGE_SIZE - 1);
    let hole_start = match arch_mm::firmware_phys_start() {
        0 => ks,
        fw => core::cmp::min(fw, ks),
    };
    let alloc = unsafe { &mut *frame_allocator::ALLOCATOR.0.get() };

    collected.each(|start, end| {
        regions::clip_region(start, end, hole_start, ke, |s, e| alloc.add_region(s, e));
    });

    heap::self_test();

    page_table::init();
    page_table::self_test();
}
