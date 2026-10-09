//! Architecture-neutral Korhv boot sequence.

use kor::config;

/// Physical extent of the korhv image (linker symbols).
fn kernel_phys_range() -> (usize, usize) {
    unsafe extern "C" {
        static _skernel: u8;
        static _end: u8;
    }
    // SAFETY: linker symbols are static addresses; only their values are read.
    let (start, end) = unsafe { (&_skernel as *const u8 as usize, &_end as *const u8 as usize) };
    let offset = crate::arch::mm::phys_to_virt(0);
    (start - offset, end - offset)
}

/// Install shared services, initialize physical memory, and enable heap growth.
pub fn initialize() {
    kor::install_console(&crate::arch::console::CONSOLE);
    config::init(crate::arch::platform_config());
    crate::heap::init_bootstrap();
    #[cfg(target_arch = "x86_64")]
    crate::arch::trap::init();
    #[cfg(target_arch = "aarch64")]
    crate::arch::hyp::trap_init();
    kor::cmdline::init_from(crate::arch::mm::boot_cmdline().unwrap_or_default());
    initialize_frames();
    crate::HEAP.enable_frame_growth();
    crate::heap::self_test();
}

fn initialize_frames() {
    let mut regions = kor::regions::RegionCollector::new();
    crate::arch::mm::detect_memory_regions(&mut |start, end| regions.add(start, end));

    let (kernel_start, kernel_end) = kernel_phys_range();
    let kernel_start = align_down(kernel_start);
    let kernel_end = align_up(kernel_end);
    let reserved_start = match config::firmware_phys_start() {
        0 => kernel_start,
        firmware_start => core::cmp::min(firmware_start, kernel_start),
    };

    regions.each(|start, end| {
        kor::regions::clip_region(
            start,
            end,
            reserved_start,
            kernel_end,
            crate::arch::add_free_region,
        );
    });
}

fn align_down(address: usize) -> usize {
    address & !(kor::frame::PAGE_SIZE - 1)
}

fn align_up(address: usize) -> usize {
    (address + kor::frame::PAGE_SIZE - 1) & !(kor::frame::PAGE_SIZE - 1)
}
