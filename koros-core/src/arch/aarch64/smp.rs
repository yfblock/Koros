//! aarch64 SMP: CPU identity via `MPIDR_EL1`, secondary bring-up via PSCI
//! `CPU_ON` (HVC conduit, as advertised by the QEMU `virt` device tree).
//!
//! PSCI powers on a core at the physical address of the `_secondary_start`
//! stub (see `boot.S`), passing the per-CPU stack top as the context id.

use crate::smp::MAX_CPUS;

/// Per-secondary-CPU kernel stack size.
const SECONDARY_STACK_SIZE: usize = 0x1_0000; // 64 KiB

#[repr(align(16))]
struct Stack([u8; SECONDARY_STACK_SIZE]);

static mut SECONDARY_STACKS: [Stack; MAX_CPUS] =
    [const { Stack([0; SECONDARY_STACK_SIZE]) }; MAX_CPUS];

/// PSCI `CPU_ON` (SMC64 function id).
const PSCI_FN_CPU_ON: usize = 0xC400_0003;

const KERNEL_OFFSET: usize = 0xFFFF_0000_0000_0000;

unsafe extern "C" {
    fn _secondary_start();
}

/// Affinity/core id from `MPIDR_EL1`.
pub fn cpu_id() -> usize {
    let mpidr: usize;
    // SAFETY: reads a read-only system register.
    unsafe { core::arch::asm!("mrs {}, mpidr_el1", out(reg) mpidr) };
    mpidr & 0x00ff_ffff
}

/// No-op: the CPU id is read from hardware.
pub fn set_cpu_id(_id: usize) {}

/// Idle until an interrupt.
pub fn wait_for_interrupt() {
    // SAFETY: `wfi` is a hint instruction, always safe.
    unsafe { core::arch::asm!("wfi") };
}

/// PSCI `CPU_ON` via the HVC conduit.  Returns the PSCI status (0 = success).
fn psci_cpu_on(target: usize, entry: usize, ctx: usize) -> isize {
    let ret: isize;
    // SAFETY: SMCCC/PSCI call.  x0-x3 are call-clobbered, so bind them all as
    // outputs to force reloads across the loop in `start_secondaries`.
    unsafe {
        core::arch::asm!(
            "hvc #0",
            inlateout("x0") PSCI_FN_CPU_ON => ret,
            inlateout("x1") target => _,
            inlateout("x2") entry => _,
            inlateout("x3") ctx => _,
        );
    }
    ret
}

/// Power on every CPU other than the boot CPU.  Returns the number that
/// accepted the `CPU_ON` request.
pub fn start_secondaries() -> usize {
    let boot = cpu_id();
    let entry_phys = _secondary_start as usize - KERNEL_OFFSET;
    let mut started = 0;
    for cpu in 0..MAX_CPUS {
        if cpu == boot {
            continue;
        }
        let stack_top =
            unsafe { core::ptr::addr_of!(SECONDARY_STACKS[cpu]) as usize + SECONDARY_STACK_SIZE };
        if psci_cpu_on(cpu, entry_phys, stack_top) == 0 {
            started += 1;
        }
    }
    started
}

/// Rust entry for a secondary CPU, called from the `boot.S` stub once the MMU
/// is re-enabled and the stack is set up.
#[unsafe(no_mangle)]
extern "C" fn rust_entry_secondary(cpu_id: usize) -> ! {
    crate::smp::secondary_entry(cpu_id)
}
