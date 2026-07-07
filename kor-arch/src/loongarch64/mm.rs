use alloc::string::String;

/// loongarch64 currently runs in direct-address mode under QEMU `-kernel`
/// (paging/DMW not yet enabled — see `boot.rs`).  The CPU executes at the
/// physical load address and the high linked-address bits are ignored by the
/// hardware, so kernel VA == PA and there is no offset.
pub fn kernel_offset() -> usize {
    0
}

/// No DTB pointer is passed in a register on this platform; the fixed
/// device-tree address is supplied by the platform configuration
/// (`mm::dtb_ptr` applies it).
pub fn dtb_ptr() -> usize {
    0
}

/// Read the kernel command line from the device tree `/chosen/bootargs`.
pub fn boot_cmdline() -> Option<String> {
    let dtb = kor::config::config_dtb();
    if dtb == 0 {
        return None;
    }
    // SAFETY: `dtb` is the platform-configured DTB address; `bootargs`
    // validates the FDT magic.
    unsafe { kor::fdt::bootargs(dtb) }
}

pub fn phys_to_virt(pa: usize) -> usize {
    pa
}

pub fn virt_to_phys(va: usize) -> usize {
    va
}

/// Detect physical memory via the device tree and register with `add_region`.
pub fn init(mut add_region: impl FnMut(usize, usize)) {
    let dtb = kor::config::config_dtb();
    if dtb != 0 {
        // SAFETY: `dtb` is the platform-configured DTB address.
        unsafe {
            kor::fdt::parse_memory_regions(dtb, &mut add_region);
        }
    }
}
