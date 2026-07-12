//! aarch64 host memory helpers.  The hypervisor runs at EL2 identity-mapped
//! (VA == PA), so kernel_offset == 0 and phys_to_virt is the identity.  Memory
//! regions and the command line come from the device tree (reusing kor::fdt).

use alloc::string::String;
use core::sync::atomic::{AtomicUsize, Ordering};

pub fn kernel_offset() -> usize {
    0
}

pub fn phys_to_virt(pa: usize) -> usize {
    pa
}

pub fn virt_to_phys(va: usize) -> usize {
    va
}

pub(crate) static DTB_PTR: AtomicUsize = AtomicUsize::new(0);

pub fn set_dtb_ptr(dtb: usize) {
    DTB_PTR.store(dtb, Ordering::Relaxed);
}

pub fn dtb_ptr() -> usize {
    DTB_PTR.load(Ordering::Relaxed)
}

/// Read the kernel command line from the device tree /chosen/bootargs.
pub fn boot_cmdline() -> Option<String> {
    let dtb = DTB_PTR.load(Ordering::Relaxed);
    if dtb == 0 {
        return None;
    }
    // SAFETY: DTB_PTR was captured at boot and points to a valid FDT blob in
    // identity-mapped memory.
    unsafe { kor::fdt::bootargs(dtb) }
}

/// Detect physical memory regions via FDT and register them with add_region.
pub fn detect_memory_regions(mut add_region: impl FnMut(usize, usize)) {
    let dtb = DTB_PTR.load(Ordering::Relaxed);
    if dtb != 0 {
        // SAFETY: dtb is a valid FDT pointer.
        unsafe { kor::fdt::parse_memory_regions(dtb, &mut add_region) };
    }
}
