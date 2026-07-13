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
mod panic;
pub mod syscall;
mod user;

use kor::config::{self, Console as ConsoleCfg, PlatformConfig};
use spin::Mutex;

#[global_allocator]
static HEAP: kor_alloc::LockedSlabHeap = kor_alloc::LockedSlabHeap::empty();

fn platform_config() -> PlatformConfig {
    #[cfg(target_arch = "riscv64")]
    return PlatformConfig {
        console: ConsoleCfg::Ns16550aMmio { base: 0x1000_0000 },
        firmware_phys_start: 0x8000_0000,
        dtb: 0,
        pci: None,
    };
    #[cfg(target_arch = "aarch64")]
    return PlatformConfig {
        console: ConsoleCfg::Pl011Mmio { base: 0x0900_0000 },
        firmware_phys_start: 0x4000_0000,
        dtb: 0,
        pci: None,
    };
    #[cfg(target_arch = "x86_64")]
    return PlatformConfig {
        console: ConsoleCfg::Ns16550aPort { base: 0x3F8 },
        firmware_phys_start: 0,
        dtb: 0,
        pci: None,
    };
    #[cfg(not(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "x86_64")))]
    compile_error!("korix supports riscv64, aarch64, and x86_64");
}

struct KernelTrapCallbacks;
impl kor::TrapCallbacks for KernelTrapCallbacks {
    fn on_timer(&self) {
        kor::time::increment_tick();
        kor::arch::current().handle_tick();
        // Drain UART RX FIFO into kernel buffer to prevent overflow during user init
        #[cfg(target_arch = "riscv64")]
        crate::syscall::rx_fill();
    }
    fn on_external(&self, _irq: u32) {}
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
    heap::init_bootstrap();
    kor_fs::register_filesystem(&kor_ext2::EXT2_DRIVER);
    kor_fs::register_filesystem(&kor_ramfs::RAMFS_DRIVER);
    kor::arch::current().trap_init();
    crate::arch::trap::init();

    kor::cmdline::init_from(kor::arch::current().boot_cmdline().unwrap_or_default());
    let mut collected = kor::regions::RegionCollector::new();
    kor::arch::current().detect_memory_regions(&mut |start, end| collected.add(start, end));
    let (ks, ke) = kor::kernel_phys_range();
    let ks = ks & !(kor_frame::PAGE_SIZE - 1);
    let ke = (ke + kor_frame::PAGE_SIZE - 1) & !(kor_frame::PAGE_SIZE - 1);
    let hole_start = match config::firmware_phys_start() {
        0 => ks,
        fw => core::cmp::min(fw, ks),
    };
    collected.each(|start, end| {
        kor::regions::clip_region(start, end, hole_start, ke, |s, e| {
            kor_frame::add_region(s, e)
        });
    });
    heap::self_test();
    kor::arch::current().page_table_init();
    kor::arch::current().timer_init();
    kor::arch::current().irq_enable();

    kor::println!("");
    kor::println!("=== korix: Linux-compatible kernel ({} KiB free) ===",
        kor_frame::available_frames() * 4);
    kor::println!("cmdline: {:?}", kor::cmdline::raw());

    crate::arch::probe_devices();
    kor::smp::boot_secondaries();
    crate::fs::init_rootfs();

    // Install syscall handler so ecall from U-mode dispatches to our syscall layer.
    kor::install_syscall_handler(syscall::dispatch);

    // Split L2 identity mapping so user pages can be mapped in low address space.
    #[cfg(target_arch = "riscv64")]
    kor_arch::riscv64::page_table::split_l2_identity();

    // Drain UART RX buffer (leftover bytes from boot).
    #[cfg(target_arch = "riscv64")]
    while kor_arch::uart::ns16550a::getchar_mmio(0x1000_0000).is_some() {}

    // Verify rootfs is accessible
    match crate::fs::list_dir("/") {
        Ok(entries) => kor::println!("[korix] root / has {} entries", entries.len()),
        Err(e) => kor::println!("[korix] root / list failed: {:?}", e),
    }

    crate::user::init::spawn_init();

    // If spawn_init returns (e.g., ELF load failed), idle.
    kor::println!("[korix] init returned, entering idle loop");
    loop {
        kor::arch::current().wait_for_interrupt();
    }
}

#[unsafe(no_mangle)]
extern "C" fn secondary_entry(cpu_id: usize) -> ! {
    crate::arch::trap::init();
    let ic = kor_arch::interrupt_controller();
    ic.init();
    kor::arch::current().timer_init();
    kor::arch::current().irq_enable();
    kor::smp::register_online();
    loop {
        kor::arch::current().wait_for_interrupt();
    }
}
