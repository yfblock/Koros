#![no_std]
#![no_main]
#![allow(bad_asm_style)]
#![allow(unsafe_op_in_unsafe_fn)]
//! korix — a Linux-compatible kernel that reuses the Koros workspace crates.

extern crate alloc;

mod arch;
mod fs;
mod heap;
mod mm;
mod net;
mod panic;
mod registries;
mod sched;
pub mod syscall;
mod user;
mod virtio;

use kor::config;
use spin::Mutex;

#[global_allocator]
static HEAP: kor_alloc::LockedSlabHeap = kor_alloc::LockedSlabHeap::empty();

pub static MOUNTS: Mutex<kor_fs::mount::MountTable> = Mutex::new(kor_fs::mount::MountTable::new());

/// Per-task-switch notification for kor-sched (link-time contract):
/// on x86_64, point the native SYSCALL kernel stack at the new task.
#[unsafe(no_mangle)]
extern "C" fn sched_task_switch(user_stack_top: usize) {
    #[cfg(target_arch = "x86_64")]
    kor_arch::x86_64::trap::set_syscall_stack(user_stack_top);
    #[cfg(not(target_arch = "x86_64"))]
    let _ = user_stack_top;
}

#[unsafe(no_mangle)]
extern "C" fn kernel_main() -> ! {
    // korix runs high-half: one translation for heap, devices, and DMA.
    kor::install_translation(kor_arch::phys_to_virt, kor_arch::virt_to_phys);

    kor_arch::console_init(kor_arch::dtb_ptr());
    kor::install_console(kor_arch::console());
    kor::println!("console: base {:#x} (from FDT)", kor_arch::console_base());
    config::init(arch::platform_config());
    kor_arch::interrupt_controller().init();
    heap::init_bootstrap();
    kor_fs::register_filesystem(&kor_ext2::EXT2_DRIVER);
    kor_fs::register_filesystem(&kor_ramfs::RAMFS_DRIVER);
    arch::trap::init();

    kor::cmdline::init_from(kor_arch::boot_cmdline().unwrap_or_default());
    let mut collected = kor::regions::RegionCollector::new();
    kor_arch::detect_memory_regions(&mut |start, end| collected.add(start, end));
    let (ks, ke) = kor_arch::kernel_phys_range();
    let ks = kor::frame::PAGE_SIZE * (ks / kor::frame::PAGE_SIZE); // align-down
    let ke = ke.next_multiple_of(kor::frame::PAGE_SIZE); // align-up
    let hole_start = match config::firmware_phys_start() {
        0 => ks,
        fw => core::cmp::min(fw, ks),
    };
    collected.each(|start, end| {
        kor::regions::clip_region(start, end, hole_start, ke, |s, e| {
            kor::frame::add_region(s, e)
        });
    });
    HEAP.enable_frame_growth();
    heap::self_test();
    kor_mmu::page_table_init();
    kor_arch::timer_init();
    kor_arch::irq_enable();

    kor::println!("");
    kor::println!(
        "=== korix: Linux-compatible kernel ({} KiB free) ===",
        kor::frame::available_frames() * 4
    );
    kor::println!("cmdline: {:?}", kor::cmdline::raw());

    probe_devices();
    kor_arch::boot_secondaries();
    crate::fs::init_rootfs();

    // Install syscall handler so ecall from U-mode dispatches to our syscall layer.

    // Split identity mapping so user pages can be mapped in low address space.
    #[cfg(target_arch = "riscv64")]
    kor_mmu::split_identity();
    #[cfg(target_arch = "aarch64")]
    kor_mmu::split_identity();
    #[cfg(target_arch = "x86_64")]
    kor_mmu::split_identity();

    // Drain UART RX buffer (leftover bytes from boot).
    while kor::console::getchar().is_some() {}

    // Verify rootfs is accessible
    match crate::fs::list_dir("/") {
        Ok(entries) => kor::println!("[korix] root / has {} entries", entries.len()),
        Err(e) => kor::println!("[korix] root / list failed: {:?}", e),
    }
    crate::sched::init();

    let init = crate::sched::process(1).expect("init process missing");
    crate::sched::spawn_process(&init, crate::user::init::spawn_init);
    kor_sched::idle_loop()
}

#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
fn probe_devices() {
    use kor::driver::{DeviceDriver, probe_fdt};
    let drivers: &[&dyn DeviceDriver] = &[&virtio::VIRTIO_MMIO_DRIVER];
    let dtb = kor_arch::dtb_ptr();
    if dtb != 0 {
        kor::println!("[korix] probing FDT at {:#x}", dtb);
        probe_fdt(dtb, drivers);
    }
}

#[cfg(target_arch = "x86_64")]
fn probe_devices() {
    virtio::probe_pci_and_register();
}

#[unsafe(no_mangle)]
extern "C" fn secondary_entry(_cpu_id: usize) -> ! {
    arch::trap::init();
    let ic = kor_arch::interrupt_controller();
    ic.init();
    kor_arch::timer_init();
    kor_arch::irq_enable();
    kor::smp::register_online();
    while !kor_sched::is_ready() {
        core::hint::spin_loop();
    }
    kor_sched::init_this_cpu();
    kor_sched::idle_loop()
}
