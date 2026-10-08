#![no_std]
#![no_main]
#![allow(bad_asm_style)]
#![allow(unsafe_op_in_unsafe_fn)]
//! korix — a Linux-compatible kernel that reuses the Koros workspace crates.

extern crate alloc;

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

use kor::config::{self, PlatformConfig};
use spin::Mutex;

#[global_allocator]
static HEAP: kor_alloc::LockedSlabHeap = kor_alloc::LockedSlabHeap::empty();

fn platform_config() -> PlatformConfig {
    #[cfg(target_arch = "riscv64")]
    return PlatformConfig {
        console: config::Console::Ns16550aMmio { base: 0x1000_0000 },
        firmware_phys_start: 0x8000_0000,
        dtb: 0,
        pci: None,
    };
    #[cfg(target_arch = "aarch64")]
    return PlatformConfig {
        console: config::Console::Pl011Mmio { base: 0x0900_0000 },
        firmware_phys_start: 0x4000_0000,
        dtb: 0,
        pci: None,
    };
    #[cfg(target_arch = "x86_64")]
    return PlatformConfig {
        console: config::Console::Ns16550aPort { base: 0x3F8 },
        firmware_phys_start: 0,
        dtb: 0,
        pci: None,
    };
    #[cfg(not(any(
        target_arch = "riscv64",
        target_arch = "aarch64",
        target_arch = "x86_64"
    )))]
    compile_error!("korix supports riscv64, aarch64, and x86_64");
}

struct KernelTrapCallbacks;
impl kor::TrapCallbacks for KernelTrapCallbacks {
    fn on_timer(&self) {
        kor::time::increment_tick();
        kor::arch::current().handle_tick();
        if kor_sched::is_ready() {
            kor_sched::timer_tick();
            kor_sched::preempt();
        }
        #[cfg(target_arch = "riscv64")]
        crate::syscall::rx_fill();
    }
    fn on_external(&self, irq: u32) {
        registries::IRQS.handle(irq);
    }
}

static TRAP_CB: KernelTrapCallbacks = KernelTrapCallbacks;
pub static MOUNTS: Mutex<kor_fs::mount::MountTable> = Mutex::new(kor_fs::mount::MountTable::new());

#[unsafe(no_mangle)]
extern "C" fn kernel_main() -> ! {
    kor::install_console(kor_arch::console());
    config::init(platform_config());
    kor::install(kor_arch::provider());
    let ic = kor_arch::interrupt_controller();
    kor::install_controller(ic);
    ic.init();
    kor::install_callbacks(&TRAP_CB);
    kor::trap_callbacks::install_user_trap_hooks(
        syscall::user_trap_enter,
        syscall::user_trap_leave,
    );
    kor::trap_callbacks::install_user_trap_status_hook(syscall::user_trap_status);
    kor::trap_callbacks::install_user_exit_hook(sched::exit_current);
    #[cfg(target_arch = "x86_64")]
    kor::trap_callbacks::install_task_switch_hook(kor_arch::x86_64::trap::set_syscall_stack);
    heap::init_bootstrap();
    kor_fs::register_filesystem(&kor_ext2::EXT2_DRIVER);
    kor_fs::register_filesystem(&kor_ramfs::RAMFS_DRIVER);
    kor::arch::current().trap_init();

    kor::cmdline::init_from(kor::arch::current().boot_cmdline().unwrap_or_default());
    let mut collected = kor::regions::RegionCollector::new();
    kor::arch::current().detect_memory_regions(&mut |start, end| collected.add(start, end));
    let (ks, ke) = kor::kernel_phys_range();
    let ks = ks & !(kor::frame::PAGE_SIZE - 1);
    let ke = (ke + kor::frame::PAGE_SIZE - 1) & !(kor::frame::PAGE_SIZE - 1);
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
    kor::arch::current().timer_init();
    kor::arch::current().irq_enable();

    kor::println!("");
    kor::println!(
        "=== korix: Linux-compatible kernel ({} KiB free) ===",
        kor::frame::available_frames() * 4
    );
    kor::println!("cmdline: {:?}", kor::cmdline::raw());

    probe_devices();
    kor::smp::boot_secondaries();
    crate::fs::init_rootfs();

    // Install syscall handler so ecall from U-mode dispatches to our syscall layer.
    kor::install_syscall_handler(syscall::dispatch);

    // Split identity mapping so user pages can be mapped in low address space.
    #[cfg(target_arch = "riscv64")]
    kor_mmu::split_identity();
    #[cfg(target_arch = "aarch64")]
    kor_mmu::split_identity();
    #[cfg(target_arch = "x86_64")]
    kor_mmu::split_identity();

    // Drain UART RX buffer (leftover bytes from boot).
    #[cfg(target_arch = "riscv64")]
    while kor_arch::uart::ns16550a::getchar_mmio(0x1000_0000).is_some() {}
    #[cfg(target_arch = "aarch64")]
    while kor_arch::uart::pl011::getchar(0x0900_0000).is_some() {}
    #[cfg(target_arch = "x86_64")]
    while kor_arch::uart::ns16550a::getchar_port(0x3F8).is_some() {}

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
    let dtb = kor::arch::current().dtb_ptr();
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
    kor::arch::current().trap_init();
    let ic = kor_arch::interrupt_controller();
    ic.init();
    kor::arch::current().timer_init();
    kor::arch::current().irq_enable();
    kor::smp::register_online();
    while !kor_sched::is_ready() {
        core::hint::spin_loop();
    }
    kor_sched::init_this_cpu();
    kor_sched::idle_loop()
}
