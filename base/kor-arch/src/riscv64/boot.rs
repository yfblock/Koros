//! riscv64 boot — Sv39 page table setup, MMU enable, jump to high-half VA.

use core::arch::global_asm;

const KERNEL_OFFSET: usize = 0xFFFF_FFC0_0000_0000;

global_asm!(
    include_str!("boot.S"),
    kernel_offset = const KERNEL_OFFSET,
);

#[unsafe(no_mangle)]
extern "C" fn rust_entry(hart_id: usize, dtb: usize) {
    // Whichever hart SBI hands control to is the boot hart (not necessarily
    // hart 0).  Record its id and run the kernel; secondaries are started
    // later via `smp::boot_secondaries`.
    crate::riscv64::smp::set_cpu_id(hart_id);
    crate::riscv64::mm::set_dtb_ptr(dtb);
    // SAFETY: `kernel_main` is provided by the `koros` binary crate.
    unsafe { crate::kernel_main() }
}
