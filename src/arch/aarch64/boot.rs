//! aarch64 boot — page table setup, MMU enable, jump to high-half VA.

use core::arch::global_asm;

global_asm!(include_str!("boot.S"));

#[unsafe(no_mangle)]
extern "C" fn rust_entry(_hart_id: usize, dtb: usize) {
    crate::arch::aarch64::mm::set_dtb_ptr(dtb);
    crate::kernel_main();
}
