pub fn kernel_offset() -> usize {
    0x9000_0000_0000_0000
}

/// Detect physical memory via FDT at a fixed address and add to the allocator.
///
/// QEMU loongarch64 places the DTB at a hardcoded physical address.
pub fn init(alloc: &mut crate::mm::frame_allocator::FrameAllocator) {
    const DTB_ADDR: usize = 0x100000;
    unsafe {
        crate::mm::fdt::parse_memory_regions(DTB_ADDR, |start, end| {
            alloc.add_region(start, end);
        });
    }
}
