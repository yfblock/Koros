//! riscv64 boot fragments (uniform names — see `crate::boot`).
//!
//! Composition order: `boot_stack_asm!` → `boot_pt_buffer_asm!` →
//! `boot_setup_asm!` → *mapping policy* (result in `a0`) →
//! `boot_cross_asm!` → `boot_call_asm!(entry)` → `boot_secondary_asm!`.
use crate::kernel_main;


/// BSS symbols for the kor-mmu `boot_mmu_build` hook ABI: a two-page
/// page-table buffer plus the offset cell the secondary stub reads.
#[macro_export]
macro_rules! boot_pt_buffer_asm {
    () => {
        "
.section .bss.boot_page_table,\"aw\",@nobits
.balign 4096
.global _boot_pt_buffer
_boot_pt_buffer:
    .space 4096 * 2
.section .bss.boot_mmu_offset,\"aw\",@nobits
.balign 8
_boot_mmu_offset:
    .space 8
"
    };
}

/// `_start` prologue: save firmware args, clear BSS, switch to the boot
/// stack.  Falls through in the physical world with `s0` = hart id and
/// `s1` = DTB pointer.
#[macro_export]
macro_rules! boot_setup_asm {
    () => {
        "
.section .text.entry
.global _start
_start:
    mv      s0, a0                 # hart_id
    mv      s1, a1                 # DTB pointer

    # la is physical while the PC is physical
    la      t0, _sbss
    la      t1, _ebss
    bgeu    t0, t1, 2f
1:
    sd      zero, (t0)
    addi    t0, t0, 8
    bltu    t0, t1, 1b
2:
    la      sp, bstack_top
"
    };
}

/// Offset bookkeeping, Sv39 enable (tables at `_boot_pt_buffer`), and the
/// crossing to virtual PCs.  Expects the mapping-slot result in `a0`
/// (0 = stay unmapped); falls through with `a0`/`a1` restored and `sp`
/// offset-adjusted.
#[macro_export]
macro_rules! boot_cross_asm {
    () => {
        "
    la      t2, _boot_mmu_offset
    sd      a0, (t2)
    beqz    a0, .Lkor_boot_phys

    la      t0, _boot_pt_buffer
    srli    t0, t0, 12
    li      t1, (8 << 60)          # SATP_MODE_SV39
    or      t0, t0, t1
    csrw    satp, t0
    sfence.vma

    la      t0, .Lkor_boot_virt
    add     t0, t0, a0
    jr      t0
.Lkor_boot_virt:
    add     sp, sp, a0
.Lkor_boot_phys:
    mv      a0, s0
    mv      a1, s1
"
    };
}

/// Call the application entry (`PC`-relative, valid in both the mapped and
/// unmapped configurations) and park if it ever returns.
#[macro_export]
macro_rules! boot_call_asm {
    ($entry:ident) => {
        concat!(
            "
    call    ",
            stringify!($entry),
            "
    j       .
",
        )
    };
}

/// Secondary-hart stub (entered physically via SBI `hart_start`): re-install
/// the boot root, cross the offset, and enter `rust_entry_secondary`.
/// Requires the [`boot_pt_buffer_asm!`] symbols.
#[macro_export]
macro_rules! boot_secondary_asm {
    () => {
        "
.global _secondary_start
_secondary_start:
    mv      s0, a0                 # hart_id
    mv      s1, a1                 # stack top (post-offset)

    la      t0, _boot_mmu_offset
    ld      t2, (t0)
    beqz    t2, .Lkor_sec_phys

    la      t0, _boot_pt_buffer
    srli    t0, t0, 12
    li      t1, (8 << 60)          # SATP_MODE_SV39
    or      t0, t0, t1
    csrw    satp, t0
    sfence.vma

    la      t0, .Lkor_sec_virt
    add     t0, t0, t2
    jr      t0
.Lkor_sec_virt:
.Lkor_sec_phys:
    mv      sp, s1
    mv      tp, s0
    mv      a0, s0
    call    rust_entry_secondary
    j       .
"
    };
}

/// First Rust function of the boot flow (`call rust_entry` from the app's
/// composed trampoline): stash the firmware args into kor-arch state, then
/// enter the application's `kernel_main` (link-time extern).
#[unsafe(no_mangle)]
extern "C" fn rust_entry(hart_id: usize, dtb: usize) {
    super::smp::set_cpu_id(hart_id);
    super::mm::set_dtb_ptr(dtb);
    // SAFETY: the app binary provides kernel_main at link time; it never
    // returns.
    unsafe { kernel_main() }
}
