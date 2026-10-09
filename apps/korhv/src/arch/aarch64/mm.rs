//! aarch64 host memory helpers.
//!
//! The hypervisor runs at EL2 identity-mapped (VA == PA, single translation
//! regime), so translation here is the identity — unlike kor-arch's EL1
//! high-half back-end, which is why `phys_to_virt`/`virt_to_phys` stay
//! local.  The DTB capture and the FDT-derived helpers forward to kor-arch
//! (single source; kor-arch's `rust_entry` stashes the pointer there).

pub fn phys_to_virt(pa: usize) -> usize {
    pa
}

/// Reserved for future stage-2 mapping.
#[allow(dead_code)]
pub fn virt_to_phys(va: usize) -> usize {
    va
}

pub use kor_arch::aarch64::mm::boot_cmdline;

/// Detect physical memory regions via FDT and register them with
/// `add_region`.
pub fn detect_memory_regions(add_region: impl FnMut(usize, usize)) {
    kor_arch::aarch64::mm::init(add_region);
}
