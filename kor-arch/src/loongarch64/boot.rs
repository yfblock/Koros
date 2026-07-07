//! loongarch64 boot — minimal entry.
//!
//! TODO: Add DMW (Direct Mapping Window) and paging enable.

use core::arch::naked_asm;

#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
unsafe extern "C" fn _start() -> ! {
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

#[unsafe(no_mangle)]
extern "C" fn rust_entry() {
    // SAFETY: `kernel_main` is provided by the `koros` binary crate.
    unsafe { crate::kernel_main() }
}

/// Secondary-core entry, reached after the boot core places the entry address
/// in mailbox 0 and the stack top in mailbox 1 and sends an IPI.  Runs in
/// direct-address mode like the boot core.
#[unsafe(naked)]
#[unsafe(no_mangle)]
unsafe extern "C" fn _secondary_start() -> ! {
    naked_asm!(
        "li.w      $t0, 0x1028         # IOCSR MAIL_BUF1 (per-core)
         iocsrrd.d $sp, $t0            # stack top from mailbox 1
         csrrd     $a0, 0x20           # cpuid
         la.global $t0, rust_entry_secondary
         jirl      $zero, $t0, 0",
    )
}

#[unsafe(no_mangle)]
extern "C" fn rust_entry_secondary(cpu_id: usize) -> ! {
    unsafe { crate::secondary_entry(cpu_id) }
}
