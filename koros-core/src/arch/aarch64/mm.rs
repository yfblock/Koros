use alloc::string::String;
use core::sync::atomic::{AtomicUsize, Ordering};

pub fn kernel_offset() -> usize {
    0xffff_0000_0000_0000
}

/// Read the kernel command line from the device tree `/chosen/bootargs`.
pub fn boot_cmdline() -> Option<String> {
    let dtb = DTB_PTR.load(Ordering::Relaxed);
    if dtb == 0 {
        return None;
    }
    // SAFETY: `DTB_PTR` was captured at boot and points to a valid FDT blob.
    unsafe { crate::mm::fdt::bootargs(dtb) }
}

pub fn phys_to_virt(pa: usize) -> usize {
    pa + kernel_offset()
}

pub fn virt_to_phys(va: usize) -> usize {
    va - kernel_offset()
}

pub(crate) static DTB_PTR: AtomicUsize = AtomicUsize::new(0);

pub fn set_dtb_ptr(dtb: usize) {
    DTB_PTR.store(dtb, Ordering::Relaxed);
}

/// Platform firmware / boot reserved area starts here on virt.
pub fn firmware_phys_start() -> usize {
    0x4000_0000
}

/// Detect physical memory regions via FDT and register them with `add_region`.
pub fn init(mut add_region: impl FnMut(usize, usize)) {
    let dtb = DTB_PTR.load(Ordering::Relaxed);
    if dtb != 0 {
        unsafe {
            crate::mm::fdt::parse_memory_regions(dtb, &mut add_region);
        }
    }
}
