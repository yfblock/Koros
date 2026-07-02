//! riscv64 boot — Sv39 page table setup, MMU enable, jump to high-half VA.

use core::arch::global_asm;

const KERNEL_OFFSET: usize = 0xFFFF_FFC0_0000_0000;

global_asm!(
    include_str!("boot.S"),
    kernel_offset = const KERNEL_OFFSET,
);

#[unsafe(no_mangle)]
extern "C" fn rust_entry(hart_id: usize, dtb: usize) {
    crate::arch::riscv64::mm::set_dtb_ptr(dtb);
    if hart_id == 0 {
        // SAFETY: `kernel_main` is provided by the `koros` binary crate.
        unsafe { crate::kernel_main() }
    }
    loop {
        core::hint::spin_loop();
    }
}
