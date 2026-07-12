#![no_std]
#![no_main]
#![allow(bad_asm_style)]
#![allow(unsafe_op_in_unsafe_fn)]
//! `korhv` -- a minimal Type-1 hypervisor, composed from the same arch-neutral
//! building blocks as `koros` (`kor`, `kor-frame`, `kor-alloc`).
//!
//! Unlike `koros`, this binary crate does not bring up a filesystem, scheduler,
//! timer or SMP; it runs single-CPU with interrupts disabled, enables the
//! hardware virtualization extension (AMD SVM on x86_64, EL2 on aarch64),
//! creates one VM with a stage-2 (NPT / VTTBR) identity mapping, and runs a
//! vCPU that executes a tiny guest image using a hypercall ABI
//! (`HCALL_PUTCHAR` / `HCALL_EXIT`).

extern crate alloc;

mod arch;
mod heap;
mod panic;

#[global_allocator]
static HEAP: kor_alloc::LockedSlabHeap = kor_alloc::LockedSlabHeap::empty();

use kor::config::{self, Console as ConsoleCfg, PlatformConfig};

/// Hypercall numbers shared by both architectures (the trap instruction and
/// argument registers differ -- see each `arch::<arch>::hyp`).
pub const HCALL_PUTCHAR: usize = 1;
pub const HCALL_EXIT: usize = 2;

fn platform_config() -> PlatformConfig {
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
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    compile_error!("korhv supports only x86_64 and aarch64");
}

#[unsafe(no_mangle)]
extern "C" fn kernel_main() -> ! {
    kor::install_console(&crate::arch::console::CONSOLE);
    config::init(platform_config());
    kor::install(&crate::arch::provider::PROVIDER);
    heap::init_bootstrap();
    kor::arch::current().trap_init();

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
            #[cfg(target_arch = "aarch64")]
            kor::regions::clip_region(s, e, 0x4020_0000, 0x5000_0000, |gs, ge| {
                kor_frame::add_region(gs, ge)
            });
            #[cfg(not(target_arch = "aarch64"))]
            kor_frame::add_region(s, e);
        });
    });
    heap::self_test();

    kor::println!("");
    kor::println!("=== korhv: Type-1 hypervisor ({} KiB free) ===",
        kor_frame::available_frames() * 4);
    kor::println!("cmdline: {:?}", kor::cmdline::raw());

    crate::arch::hyp::init();
    crate::arch::hyp::run();

    kor::println!("=== korhv: guest halted, spinning ===");
    loop {
        core::hint::spin_loop();
    }
}
