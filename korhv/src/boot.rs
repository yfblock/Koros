//! Architecture-neutral Korhv boot sequence.

use kor::config;

/// Install shared services, initialize physical memory, and enable heap growth.
pub fn initialize() {
    kor::install_console(&crate::arch::console::CONSOLE);
    config::init(crate::arch::platform_config());
    kor::install(&crate::arch::provider::PROVIDER);
    crate::heap::init_bootstrap();
    kor::arch::current().trap_init();
    kor::cmdline::init_from(kor::arch::current().boot_cmdline().unwrap_or_default());
    initialize_frames();
    crate::HEAP.enable_frame_growth();
    crate::heap::self_test();
}

fn initialize_frames() {
    let mut regions = kor::regions::RegionCollector::new();
    kor::arch::current().detect_memory_regions(&mut |start, end| regions.add(start, end));

    let (kernel_start, kernel_end) = kor::kernel_phys_range();
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
    address & !(kor_frame::PAGE_SIZE - 1)
}

fn align_up(address: usize) -> usize {
    (address + kor_frame::PAGE_SIZE - 1) & !(kor_frame::PAGE_SIZE - 1)
}
