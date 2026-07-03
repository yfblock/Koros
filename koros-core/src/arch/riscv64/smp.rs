//! riscv64 SMP: hart identity via `tp`, secondary bring-up via the SBI HSM
//! extension.
//!
//! QEMU + OpenSBI starts only the boot hart in the kernel; the others wait in
//! firmware until `sbi_hart_start` jumps them to the physical address of the
//! `_secondary_start` stub (see `boot.S`), which enables paging, sets the
//! per-hart stack, and calls [`rust_entry_secondary`].

use crate::smp::MAX_CPUS;

/// Per-secondary-hart kernel stack size.
const SECONDARY_STACK_SIZE: usize = 0x1_0000; // 64 KiB

#[repr(align(16))]
struct Stack([u8; SECONDARY_STACK_SIZE]);

static mut SECONDARY_STACKS: [Stack; MAX_CPUS] =
    [const { Stack([0; SECONDARY_STACK_SIZE]) }; MAX_CPUS];

// SBI Hart State Management extension.
const SBI_EXT_HSM: usize = 0x48534D;
const SBI_FN_HART_START: usize = 0;

unsafe extern "C" {
    fn _secondary_start();
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
    let start_phys = crate::mm::virt_to_phys(_secondary_start as usize);
    let mut started = 0;
    for hart in 0..MAX_CPUS {
        if hart == boot {
            continue;
        }
        // Pass the per-hart stack top (a kernel VA) as the SBI opaque value;
        // the stub loads it into `sp` only after paging is enabled.
        let stack_top =
            unsafe { core::ptr::addr_of!(SECONDARY_STACKS[hart]) as usize + SECONDARY_STACK_SIZE };
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
    crate::smp::secondary_entry(hart_id)
}
