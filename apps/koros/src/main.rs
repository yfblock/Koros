#![no_std]
#![no_main]
#![allow(bad_asm_style)]
//! Kernel composition root: installs the arch provider, interrupt controller,
//! console and trap callbacks, brings up memory/SMP/drivers, runs the ext2
//! self-check, and spawns the scheduler demo tasks.

extern crate alloc;

/// Global allocator (slab heap); initialised by `heap::init_bootstrap`.
#[global_allocator]
static HEAP: kor_alloc::LockedSlabHeap = kor_alloc::LockedSlabHeap::empty();

// The storage benchmark runs on riscv64/x86_64 only (see `kernel_main`).
mod arch;
#[cfg(any(target_arch = "riscv64", target_arch = "x86_64"))]
mod bench;
mod ext2_test;
mod heap;
mod panic;
mod registries;
mod virtio;

use kor::cmdline;
use kor::config;

/// Kernel entry point, called by the `kor-arch` boot code after early setup.
/// Per-task-switch notification for kor-sched (link-time contract):
/// no-op — koros has no user stacks to install.
#[unsafe(no_mangle)]
extern "C" fn sched_task_switch(_user_stack_top: usize) {}

#[unsafe(no_mangle)]
extern "C" fn kernel_main() -> ! {
    // 1. Console (before any println).
    kor::install_console(kor_arch::console());
    // 2. Platform config (firmware region, dtb, pci — read back by mm/driver probe).
    config::init(arch::platform_config());
    // 4. Interrupt controller (+ per-CPU init; GIC on aarch64).
    kor_arch::interrupt_controller().init();
    // 5. Trap callbacks (timer tick + external-IRQ dispatch).

    // 6. Bootstrap heap (frame buddy's internal BTreeSet needs it).
    heap::init_bootstrap();

    // Register filesystem drivers (ext2, ramfs) — mounted by name later.
    kor_fs::register_filesystem(&kor_ext2::EXT2_DRIVER);
    kor_fs::register_filesystem(&kor_ramfs::RAMFS_DRIVER);
    // 7. Trap vectors.
    arch::trap::init();
    // 8. Memory init: capture cmdline, detect regions, clip the kernel image,
    //    feed the frame allocator, self-test, then dynamic page tables.
    kor::cmdline::init_from(kor_arch::boot_cmdline().unwrap_or_default());
    let mut collected = kor::regions::RegionCollector::new();
    kor_arch::detect_memory_regions(&mut |start, end| collected.add(start, end));
    let (ks, ke) = kernel_phys_range();
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
    // Install this link model's translation (identity on the unmapped
    // arches; high-half on x86_64), then allow frame-backed heap growth.
    #[cfg(target_arch = "x86_64")]
    kor::install_translation(kor_arch::phys_to_virt, kor_arch::virt_to_phys);
    #[cfg(not(target_arch = "x86_64"))]
    kor::install_translation(|pa| pa, |va| va);
    HEAP.enable_frame_growth();
    heap::self_test();
    // 9. Timer + interrupts on this CPU.
    kor_arch::timer_init();
    kor_arch::irq_enable();

    kor::println!("Hello, world!");
    kor::println!("cmdline: {:?}", cmdline::raw());

    // 10. Discover and bind drivers.
    probe_devices();

    // 11. Bring up the other CPUs.
    let online = kor_arch::boot_secondaries();
    kor::println!(
        "SMP: {} CPU(s) online (boot cpu {})",
        online,
        kor_arch::cpu_id()
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
        kor_arch::shutdown();
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

/// No-op boot-root reactivation for the riscv64 shutdown fallback: koros
/// runs unmapped, so the syscon is always reachable at its physical address.
#[cfg(target_arch = "riscv64")]
#[unsafe(no_mangle)]
unsafe extern "C" fn boot_mmu_activate() {}

/// koros runs without page tables: every task shares the one (absent)
/// address space, so the scheduler's root hooks are identity stubs.
#[unsafe(no_mangle)]
extern "C" fn sched_address_space_root() -> usize {
    0
}

/// No-op activation (no MMU).
///
/// # Safety
/// Trivially safe: never touches translation state.
#[unsafe(no_mangle)]
unsafe extern "C" fn sched_activate_address_space(_root: usize) {}

/// Physical extent of the kernel image (linker symbols).  koros links
/// identity on riscv64/aarch64 (no MMU); x86_64 keeps the high-half link on
/// the static boot tables, so its VA→PA offset is subtracted.
fn kernel_phys_range() -> (usize, usize) {
    unsafe extern "C" {
        static _skernel: u8;
        static _end: u8;
    }
    // SAFETY: linker symbols are static addresses; only their values are read.
    let (start, end) = unsafe { (&_skernel as *const u8 as usize, &_end as *const u8 as usize) };
    #[cfg(target_arch = "x86_64")]
    let offset = 0xffff_8000_0000_0000usize;
    #[cfg(not(target_arch = "x86_64"))]
    let offset = 0;
    (start - offset, end - offset)
}

/// Secondary-CPU entry, called from the `kor-arch` secondary boot stub.
#[unsafe(no_mangle)]
extern "C" fn secondary_entry(cpu_id: usize) -> ! {
    arch::trap::init();
    kor::smp::register_online();
    kor::println!("cpu {} online", cpu_id);
    // Per-CPU interrupt controller init (GIC CPU interface on aarch64).
    kor_arch::interrupt_controller().init();
    kor_arch::timer_init();
    kor_arch::irq_enable();
    while !kor_sched::is_ready() {
        kor_arch::wait_for_interrupt();
    }
    kor_sched::init_this_cpu();
    kor_sched::idle_loop();
}

// ---------------------------------------------------------------------------
// Driver probing
// ---------------------------------------------------------------------------

#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
fn probe_devices() {
    use kor::driver::{DeviceDriver, probe_fdt};
    use virtio::VIRTIO_MMIO_DRIVER;

    static DRIVERS: &[&dyn DeviceDriver] = &[&VIRTIO_MMIO_DRIVER];
    // Config-aware DTB pointer: fixed platform address if set, else the
    // firmware-passed register captured at boot.
    let fdt = {
        let c = config::config_dtb();
        if c != 0 { c } else { kor_arch::dtb_ptr() }
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
                kor_arch::cpu_id(),
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
        kor::println!("[producer cpu{}] sent {}", kor_arch::cpu_id(), i * 10);
    }
}

fn demo_consumer() {
    for _ in 0..5 {
        let v = DEMO_CHAN.recv();
        let mut total = DEMO_TOTAL.lock();
        *total += v;
        kor::println!(
            "[consumer cpu{}] recv {} (total {})",
            kor_arch::cpu_id(),
            v,
            *total
        );
    }
    kor::println!("[consumer] done");
}

fn busy_work() {
    let start = kor_arch::ticks();
    while kor_arch::ticks() < start + 15 {
        core::hint::spin_loop();
    }
}

fn demo_task_c() {
    for round in 0..6 {
        busy_work();
        kor::println!("[task C cpu{}] round {}", kor_arch::cpu_id(), round);
    }
    kor::println!("[task C] done");
}

fn demo_task_d() {
    for round in 0..6 {
        busy_work();
        kor::println!("[task D cpu{}] round {}", kor_arch::cpu_id(), round);
    }
    kor::println!("[task D] done");
}

fn demo_task_a() {
    for i in 0..5 {
        kor::println!("[task A cpu{}] iteration {}", kor_arch::cpu_id(), i);
        kor_sched::sleep_ms(200);
    }
    kor::println!("[task A] done");
}

fn demo_task_b() {
    for i in 0..8 {
        kor::println!("[task B cpu{}] tick {}", kor_arch::cpu_id(), i);
        kor_sched::yield_now();
        kor_sched::sleep_ms(120);
    }
    kor::println!("[task B] done");
}
