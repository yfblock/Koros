//! aarch64 boot — page table setup, MMU enable, jump to high-half VA.

use core::arch::global_asm;

global_asm!(include_str!("boot.S"));

#[unsafe(no_mangle)]
extern "C" fn rust_entry() {
    crate::kernel_main();
}
