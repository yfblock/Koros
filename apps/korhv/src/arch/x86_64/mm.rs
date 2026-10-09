//! x86_64 host memory helpers — forwarded to kor-arch (single source of
//! truth for the Multiboot info capture and the memory-map walk).

pub use kor_arch::x86_64::mm::{boot_cmdline, phys_to_virt, virt_to_phys};

/// Detect free memory regions through the Multiboot1 memory map.
pub fn detect_memory_regions(add_region: impl FnMut(usize, usize)) {
    kor_arch::x86_64::mm::init(add_region);
}
