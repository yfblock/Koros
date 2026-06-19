//! loongarch64 boot — minimal entry.
//!
//! TODO: Add DMW (Direct Mapping Window) and paging enable.

use core::arch::naked_asm;

#[naked]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
unsafe extern "C" fn _start() -> ! {
    unsafe {
        naked_asm!(
            "la.global $sp, bstack_top
             # --- Clear BSS ---
             la.global $t0, _sbss
             la.global $t1, _ebss
             bgeu     $t0, $t1, 2f
         1:
             st.d     $zero, $t0, 0
             addi.d   $t0, $t0, 8
             bltu     $t0, $t1, 1b
         2:
             la.global $t0, {entry}
             jirl      $zero, $t0, 0",
            entry = sym rust_entry,
        )
    }
}

#[unsafe(no_mangle)]
extern "C" fn rust_entry() {
    crate::kernel_main();
}
