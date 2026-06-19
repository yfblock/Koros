use core::sync::atomic::{AtomicUsize, Ordering};

pub fn kernel_offset() -> usize {
    0xffff_0000_0000_0000
}

pub(crate) static DTB_PTR: AtomicUsize = AtomicUsize::new(0);

pub fn set_dtb_ptr(dtb: usize) {
    DTB_PTR.store(dtb, Ordering::Relaxed);
}

/// Detect physical memory regions via FDT and add them to the allocator.
pub fn init(alloc: &mut crate::mm::frame_allocator::FrameAllocator) {
    let dtb = DTB_PTR.load(Ordering::Relaxed);
    if dtb != 0 {
        unsafe {
            crate::mm::fdt::parse_memory_regions(dtb, |start, end| {
                alloc.add_region(start, end);
            });
        }
    }
}
