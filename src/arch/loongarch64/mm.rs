pub fn kernel_offset() -> usize {
    0x9000_0000_0000_0000
}

pub fn phys_to_virt(pa: usize) -> usize {
    pa
}

pub fn virt_to_phys(va: usize) -> usize {
    va
}

pub fn firmware_phys_start() -> usize {
    0x8000_0000
}

/// Detect physical memory via FDT at a fixed address and register with `add_region`.
///
/// QEMU loongarch64 places the DTB at a hardcoded physical address.
pub fn init(mut add_region: impl FnMut(usize, usize)) {
    const DTB_ADDR: usize = 0x100000;
    unsafe {
        crate::mm::fdt::parse_memory_regions(DTB_ADDR, &mut add_region);
    }
}
