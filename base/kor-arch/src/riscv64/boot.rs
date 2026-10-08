//! riscv64 boot — generic MMU trampoline (see `boot.S`).
//!
//! The application provides `boot_mmu_build`, which fills the boot table
//! buffer and returns the phys→virt offset; this module only wires the
//! trampoline and hands control to `kernel_main`.

use core::arch::global_asm;

global_asm!(include_str!("boot.S"));

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
