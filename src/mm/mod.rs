//! Physical memory management.
//!
//! Responsibilities:
//! - Detect available physical memory (per-arch code in `src/arch/<arch>/mm.rs`).
//! - Exclude the kernel image itself from the free pool.
//! - Initialise the [`frame_allocator`] so the rest of the kernel can
//!   allocate and free physical 4 KiB frames.

pub mod frame_allocator;

// Shared FDT parser — used by riscv64, aarch64, and loongarch64.
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

// ---------------------------------------------------------------------------
// Initialisation
// ---------------------------------------------------------------------------

/// Initialise physical-memory management.
///
/// Must be called once, early in `kernel_main`, before any allocation.
pub fn init() {
    let alloc = unsafe { &mut *frame_allocator::ALLOCATOR.0.get() };

    // 1. Arch-specific memory-region detection (FDT, Multiboot, etc.).
    arch_mm::init(alloc);

    // 2. Punch out the kernel image.
    let (ks, ke) = kernel_phys_range();
    alloc.reserve(ks, ke);

    if cfg!(debug_assertions) {
        crate::println!("mm: kernel phys {:#010x}–{:#010x}", ks, ke);
        crate::println!("mm: {} free frames available", alloc.available_frames());
    }
}
