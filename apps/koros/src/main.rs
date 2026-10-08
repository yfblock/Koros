#![no_std]
#![no_main]
//! Kernel composition root: installs the arch provider, interrupt controller,
//! console and trap callbacks, brings up memory/SMP/drivers, runs the ext2
//! self-check, and spawns the scheduler demo tasks.

extern crate alloc;

/// Global allocator (slab heap); initialised by `heap::init_bootstrap`.
#[global_allocator]
static HEAP: kor_alloc::LockedSlabHeap = kor_alloc::LockedSlabHeap::empty();

// The storage benchmark runs on riscv64/x86_64 only (see `kernel_main`).
#[cfg(any(target_arch = "riscv64", target_arch = "x86_64"))]
mod bench;
mod boot_mmu;
mod ext2_test;
mod heap;
mod mmu_test;
mod panic;
mod registries;
mod virtio;

use kor::cmdline;
#[cfg(target_arch = "loongarch64")]
use kor::config::PciEcam;
use kor::config::{self, Console as ConsoleCfg, PlatformConfig};

/// Board / platform configuration for the QEMU targets.
fn platform_config() -> PlatformConfig {
    #[cfg(target_arch = "riscv64")]
    return PlatformConfig {
        console: ConsoleCfg::Ns16550aMmio { base: 0x1000_0000 },
        firmware_phys_start: 0x8000_0000,
        dtb: 0, // passed by OpenSBI in a1
        pci: None,
    };
    #[cfg(target_arch = "aarch64")]
    return PlatformConfig {
        console: ConsoleCfg::Pl011Mmio { base: 0x0900_0000 },
        firmware_phys_start: 0x4000_0000,
        dtb: 0, // passed in x0
        pci: None,
    };
    #[cfg(target_arch = "loongarch64")]
    return PlatformConfig {
        console: ConsoleCfg::Ns16550aMmio { base: 0x1FE0_01E0 },
        firmware_phys_start: 0x8000_0000,
        dtb: 0x100000, // fixed QEMU address (no register-passed pointer)
        pci: Some(PciEcam {
            ecam_base: 0x2000_0000,
            mmio_base: 0x4000_0000,
            mmio_size: 0x4000_0000,
        }),
    };
    #[cfg(target_arch = "x86_64")]
    return PlatformConfig {
        console: ConsoleCfg::Ns16550aPort { base: 0x3F8 },
        firmware_phys_start: 0,
        dtb: 0,
        pci: None,
    };
}

/// Timer + external-IRQ behaviour installed as the trap callbacks.
struct KernelTrapCallbacks;
impl kor::TrapCallbacks for KernelTrapCallbacks {
    fn on_timer(&self) {
        kor::time::increment_tick();
        kor::arch::current().handle_tick();
        kor_sched::timer_tick();
        kor_sched::preempt();
    }
    fn on_external(&self, irq: u32) {
        registries::IRQS.handle(irq);
    }
}
static TRAP_CB: KernelTrapCallbacks = KernelTrapCallbacks;

/// Kernel entry point, called by the `kor-arch` boot code after early setup.
#[unsafe(no_mangle)]
extern "C" fn kernel_main() -> ! {
    // 1. Console (before any println).
    kor::install_console(kor_arch::console());
    // 2. Platform config (firmware region, dtb, pci — read back by mm/driver probe).
    config::init(platform_config());
    // 3. Arch provider.
    kor::install(kor_arch::provider());
    // 4. Interrupt controller (+ per-CPU init; GIC on aarch64).
    let ic = kor_arch::interrupt_controller();
    kor::install_controller(ic);
    ic.init();
    // 5. Trap callbacks (timer tick + external-IRQ dispatch).
    kor::install_callbacks(&TRAP_CB);

    // 6. Bootstrap heap (frame buddy's internal BTreeSet needs it).
    heap::init_bootstrap();

    // Register filesystem drivers (ext2, ramfs) — mounted by name later.
    kor_fs::register_filesystem(&kor_ext2::EXT2_DRIVER);
    kor_fs::register_filesystem(&kor_ramfs::RAMFS_DRIVER);
    // 7. Trap vectors.
    kor::arch::current().trap_init();
    // 8. Memory init: capture cmdline, detect regions, clip the kernel image,
    //    feed the frame allocator, self-test, then dynamic page tables.
    kor::cmdline::init_from(kor::arch::current().boot_cmdline().unwrap_or_default());
    let mut collected = kor::regions::RegionCollector::new();
    kor::arch::current().detect_memory_regions(&mut |start, end| collected.add(start, end));
    let (ks, ke) = kor::kernel_phys_range();
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
    // Slab growth and large allocations may now draw from the frame buddy.
    HEAP.enable_frame_growth();
    heap::self_test();
    kor_mmu::page_table_init();
    page_table_self_test();
    if cmdline::has_flag("mmutest") {
        mmu_test::run();
    }
    // 9. Timer + interrupts on this CPU.
    kor::arch::current().timer_init();
    kor::arch::current().irq_enable();

    kor::println!("Hello, world!");
    kor::println!("cmdline: {:?}", cmdline::raw());

    // 10. Discover and bind drivers.
    probe_devices();

    // 11. Bring up the other CPUs.
    let online = kor::smp::boot_secondaries();
    kor::println!(
        "SMP: {} CPU(s) online (boot cpu {})",
        online,
        kor::arch::current().cpu_id()
    );

    // `bench` on the command line runs the storage benchmark instead of the
    // functional self-check.
    #[cfg(any(target_arch = "riscv64", target_arch = "x86_64"))]
    if cmdline::has_flag("bench") {
        bench::run();
        loop {
            core::hint::spin_loop();
        }
    }

    ext2_test::ext2_test();

    // `shutdown` on the command line powers off once the self-checks pass.
    if cmdline::has_flag("shutdown") {
        kor::println!("powering off...");
        kor::arch::current().shutdown();
    }

    // 12. Scheduler demo tasks, then become the idle task.
    kor_sched::init();
    kor_sched::spawn(demo_task_a);
    kor_sched::spawn(demo_task_b);
    kor_sched::spawn(demo_task_c);
    kor_sched::spawn(demo_task_d);
    kor_sched::spawn(demo_producer);
    kor_sched::spawn(demo_consumer);
    kor_sched::spawn(demo_blkio);
    kor_sched::idle_loop();
}

/// Exercise frame allocation + dynamic page mapping on the active root.
fn page_table_self_test() {
    use kor::{MapSize, MappingFlags};
    if !kor_mmu::dynamic_maps_supported() {
        kor::println!("mm: page-table self-test skipped (no dynamic maps)");
        return;
    }
    // Self-test fixtures: canonical low-half scratch slots, free in the boot
    // tables of every paging arch (Sv39 low half tops out at 256 GiB).
    const TEST_VA_4K: usize = 0x0000_0030_0000_0000;
    const TEST_VA_2M: usize = 0x0000_0030_0020_0000;
    const MARK: u64 = 0xDEAD_BEEF_CAFE_BABE;
    let phys = kor::frame::alloc_frame().expect("alloc_frame");
    let va = TEST_VA_4K;
    kor_mmu::map(va, phys, MappingFlags::KERNEL_RWX, MapSize::Page4K).expect("map 4K");
    unsafe {
        (va as *mut u64).write_volatile(MARK);
        assert!((va as *const u64).read_volatile() == MARK);
    }
    assert_eq!(kor_mmu::translate(va), Some(phys));

    let phys2m = kor::frame::alloc_huge_2m().expect("alloc_huge_2m");
    // Sv39 L1 megapages take PA[20:12] from VA[20:12]; match those bits.
    let va2m = (TEST_VA_2M & !0x1FF_000) | (phys2m & 0x1FF_000);
    kor_mmu::map(va2m, phys2m, MappingFlags::KERNEL_RWX, MapSize::Page2M).expect("map 2M");
    assert_eq!(kor_mmu::translate(va2m), Some(phys2m));
    unsafe {
        (va2m as *mut u64).write_volatile(MARK);
        assert!((va2m as *const u64).read_volatile() == MARK);
    }
    assert_eq!(kor_mmu::translate(va2m), Some(phys2m));
    kor::println!("mm: page-table OK (4K {:#x}, 2M {:#x})", va, va2m);
}

/// Secondary-CPU entry, called from the `kor-arch` secondary boot stub.
#[unsafe(no_mangle)]
extern "C" fn secondary_entry(cpu_id: usize) -> ! {
    kor::arch::current().trap_init();
    kor::smp::register_online();
    kor::println!("cpu {} online", cpu_id);
    // Per-CPU interrupt controller init (GIC CPU interface on aarch64).
    if let Some(ic) = kor::controller() {
        ic.init();
    }
    kor::arch::current().timer_init();
    kor::arch::current().irq_enable();
    while !kor_sched::is_ready() {
        kor::arch::current().wait_for_interrupt();
    }
    kor_sched::init_this_cpu();
    kor_sched::idle_loop();
}

// ---------------------------------------------------------------------------
// Driver probing
// ---------------------------------------------------------------------------

#[cfg(any(
    target_arch = "riscv64",
    target_arch = "aarch64",
    target_arch = "loongarch64"
))]
fn probe_devices() {
    use kor::driver::{DeviceDriver, probe_fdt};
    use virtio::VIRTIO_MMIO_DRIVER;

    static DRIVERS: &[&dyn DeviceDriver] = &[&VIRTIO_MMIO_DRIVER];
    // Config-aware DTB pointer: fixed platform address if set, else the
    // firmware-passed register captured at boot.
    let fdt = {
        let c = config::config_dtb();
        if c != 0 {
            c
        } else {
            kor::arch::current().dtb_ptr()
        }
    };
    probe_fdt(fdt, DRIVERS);

    if let Some(pci) = config::pci_ecam() {
        virtio::probe_pci_ecam_and_register(pci.ecam_base, pci.mmio_base, pci.mmio_size);
    }
}

#[cfg(target_arch = "x86_64")]
fn probe_devices() {
    virtio::probe_pci_and_register();
}

// ---------------------------------------------------------------------------
// Scheduler demo tasks
// ---------------------------------------------------------------------------

fn demo_blkio() {
    let Some(dev) = registries::BLOCKS.first() else {
        kor::println!("[blkio] no block device");
        return;
    };
    let mut buf = [0u8; 512];
    for id in 0..3 {
        match dev.read_block(id, &mut buf) {
            Ok(()) => kor::println!(
                "[blkio cpu{}] read block {} ok (byte0={:#04x})",
                kor::arch::current().cpu_id(),
                id,
                buf[0]
            ),
            Err(_) => kor::println!("[blkio] read block {} failed", id),
        }
    }
    kor::println!("[blkio] done");
}

static DEMO_CHAN: kor_sched::sync::Channel<u64> = kor_sched::sync::Channel::new();
static DEMO_TOTAL: kor_sched::sync::Mutex<u64> = kor_sched::sync::Mutex::new(0);

fn demo_producer() {
    for i in 0..5 {
        kor_sched::sleep_ms(150);
        DEMO_CHAN.send(i * 10);
        kor::println!(
            "[producer cpu{}] sent {}",
            kor::arch::current().cpu_id(),
            i * 10
        );
    }
}

fn demo_consumer() {
    for _ in 0..5 {
        let v = DEMO_CHAN.recv();
        let mut total = DEMO_TOTAL.lock();
        *total += v;
        kor::println!(
            "[consumer cpu{}] recv {} (total {})",
            kor::arch::current().cpu_id(),
            v,
            *total
        );
    }
    kor::println!("[consumer] done");
}

fn busy_work() {
    let start = kor::time::ticks();
    while kor::time::ticks() < start + 15 {
        core::hint::spin_loop();
    }
}

fn demo_task_c() {
    for round in 0..6 {
        busy_work();
        kor::println!(
            "[task C cpu{}] round {}",
            kor::arch::current().cpu_id(),
            round
        );
    }
    kor::println!("[task C] done");
}

fn demo_task_d() {
    for round in 0..6 {
        busy_work();
        kor::println!(
            "[task D cpu{}] round {}",
            kor::arch::current().cpu_id(),
            round
        );
    }
    kor::println!("[task D] done");
}

fn demo_task_a() {
    for i in 0..5 {
        kor::println!(
            "[task A cpu{}] iteration {}",
            kor::arch::current().cpu_id(),
            i
        );
        kor_sched::sleep_ms(200);
    }
    kor::println!("[task A] done");
}

fn demo_task_b() {
    for i in 0..8 {
        kor::println!("[task B cpu{}] tick {}", kor::arch::current().cpu_id(), i);
        kor_sched::yield_now();
        kor_sched::sleep_ms(120);
    }
    kor::println!("[task B] done");
}
