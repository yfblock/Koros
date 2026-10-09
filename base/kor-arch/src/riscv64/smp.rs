//! riscv64 SMP: hart identity via `tp`, secondary bring-up via the SBI HSM
//! extension.
//!
//! QEMU + OpenSBI starts only the boot hart in the kernel; the others wait in
//! firmware until `sbi_hart_start` jumps them to the physical address of the
//! `_secondary_start` stub (see `boot.S`), which enables paging, sets the
//! per-hart stack, and calls [`rust_entry_secondary`].

use kor::smp::MAX_CPUS;

/// Per-secondary-hart kernel stack size.
const SECONDARY_STACK_SIZE: usize = 0x1_0000; // 64 KiB

#[repr(align(16))]
struct Stack([u8; SECONDARY_STACK_SIZE]);

static mut SECONDARY_STACKS: [Stack; MAX_CPUS] =
    [const { Stack([0; SECONDARY_STACK_SIZE]) }; MAX_CPUS];

// SBI Hart State Management extension.
const SBI_EXT_HSM: usize = 0x48534D;
const SBI_FN_HART_START: usize = 0;
// SBI System Reset extension (firmware poweroff; OpenSBI exposes it as `srst`).
const SBI_EXT_SRST: usize = 0x5352_5454;
const SBI_FN_SYSTEM_RESET: usize = 0;
const SBI_SRST_TYPE_SHUTDOWN: usize = 0;
const SBI_SRST_REASON_NONE: usize = 0;

/// QEMU `virt` syscon poweroff register (`sifive_test`, DTB `syscon-poweroff`).
const SIFIVE_TEST_BASE: usize = 0x0010_0000;
/// `FINISHER_PASS` value: power the machine off successfully.
const SIFIVE_TEST_FINISH_PASS: u32 = 0x5555;

unsafe extern "C" {
    fn _secondary_start();
    /// Reactivate the boot page-table root; provided by kor-mmu at link
    /// time (its `boot_mmu_activate`, mirroring `boot_mmu_build`).
    fn boot_mmu_activate();
}

/// Hart id of the current CPU (kept in `tp`).
pub fn cpu_id() -> usize {
    let tp: usize;
    // SAFETY: reads the thread pointer, which holds this hart's id.
    unsafe { core::arch::asm!("mv {}, tp", out(reg) tp) };
    tp
}

/// Record this hart's id in `tp` (called once at boot on each hart).
pub fn set_cpu_id(id: usize) {
    // SAFETY: `tp` is reserved for per-CPU identity in this kernel.
    unsafe { core::arch::asm!("mv tp, {}", in(reg) id) };
}

/// Idle until an interrupt (used by parked secondary CPUs).
pub fn wait_for_interrupt() {
    // SAFETY: `wfi` is a hint instruction, always safe.
    unsafe { core::arch::asm!("wfi") };
}

/// Power the machine off.
///
/// SBI SRST `system_reset(shutdown)` is tried first (the portable path). Some
/// QEMU/OpenSBI builds advertise `srst` in their banner yet reject the call,
/// so on any return fall back to the QEMU `virt` syscon poweroff — the same
/// register OpenSBI itself would write — and finally park forever.
pub fn shutdown() -> ! {
    let err: isize;
    // SAFETY: SBI ecall per the SRST extension; a successful shutdown never
    // returns, and the clobbered a0/a1 are irrelevant afterwards.
    unsafe {
        core::arch::asm!(
            "ecall",
            inlateout("a0") SBI_SRST_TYPE_SHUTDOWN => err,
            in("a1") SBI_SRST_REASON_NONE,
            in("a6") SBI_FN_SYSTEM_RESET,
            in("a7") SBI_EXT_SRST,
        );
    }
    if err != 0 {
        // User address spaces drop the low identity map, so switch back to
        // the boot root before the MMIO store (this path never returns).
        // Link-time hook — no crate dependency between kor-arch and kor-mmu.
        // SAFETY: shutdown never resumes execution on the old root.
        unsafe { boot_mmu_activate() };
        kor::mmio::Reg::<u32>::at_pa(SIFIVE_TEST_BASE).write(SIFIVE_TEST_FINISH_PASS);
    }
    loop {
        // SAFETY: `wfi` is a hint instruction, always safe.
        unsafe { core::arch::asm!("wfi") };
    }
}

/// SBI `hart_start(hartid, start_addr, opaque)`; returns the SBI error code.
fn sbi_hart_start(hartid: usize, start_addr: usize, opaque: usize) -> isize {
    let err: isize;
    // SAFETY: SBI ecall per the HSM extension calling convention.  Both a0 and
    // a1 are clobbered by SBI (error/value return), so mark a1 as out too —
    // otherwise the compiler would keep a stale `start_addr` in a1 across
    // successive calls in the loop.
    unsafe {
        core::arch::asm!(
            "ecall",
            inlateout("a0") hartid => err,
            inlateout("a1") start_addr => _,
            in("a2") opaque,
            in("a6") SBI_FN_HART_START,
            in("a7") SBI_EXT_HSM,
        );
    }
    err
}

/// Start every hart other than the boot hart.  Returns the number of harts
/// successfully requested to start.
pub fn start_secondaries() -> usize {
    let boot = cpu_id();
    let start_phys = super::mm::virt_to_phys(_secondary_start as *const () as usize);
    let mut started = 0;
    for hart in 0..MAX_CPUS {
        if hart == boot {
            continue;
        }
        // Pass the per-hart stack top (a kernel VA) as the SBI opaque value;
        // the stub loads it into `sp` only after paging is enabled.
        let stack_top = unsafe {
            core::ptr::addr_of!(SECONDARY_STACKS[hart].0) as usize + SECONDARY_STACK_SIZE
        };
        if sbi_hart_start(hart, start_phys, stack_top) == 0 {
            started += 1;
        }
    }
    started
}

/// Rust entry for a secondary hart, called from the `boot.S` stub once paging
/// and the stack are set up.  `tp` already holds the hart id.
#[unsafe(no_mangle)]
extern "C" fn rust_entry_secondary(hart_id: usize) -> ! {
    unsafe { crate::secondary_entry(hart_id) }
}
