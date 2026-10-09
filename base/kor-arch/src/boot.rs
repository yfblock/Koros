//! Boot authoring toolkit — opt-in macros that generate the standard boot
//! trampolines and entry wrappers, so applications own small Rust files
//! instead of raw assembly copies.
//!
//! Ownership model: the *files* and all policy (stack size, MMU builder,
//! board constants) stay in the application; these macros contribute the
//! arch *mechanism* (BSS clear, stack switch, builder hook call, MMU
//! enable, offset jump).  Everything a macro emits is what the stock
//! trampoline has looked like since the app-driven MMU protocol — an
//! application wanting a different shape simply writes raw `global_asm!`
//! instead of invoking the macro.
//!
//! Typical application boot module:
//!
//! ```ignore
//! kor_arch::boot_stack!(0x80000);                    // 512 KiB, app policy
//! kor_arch::boot_trampoline_riscv64!(bstack_top, rust_entry); // mechanism
//! kor_arch::boot_entry_riscv64!();                   // rust_entry wrapper
//! ```
//!
//! The default boot mapping (and the `boot_mmu_build` hook the trampoline
//! calls) is provided by kor-mmu per arch; an application wanting a
//! different mapping or boot shape writes raw `global_asm!` instead.

/// Define the boot stack in `.bss.bstack` (size in bytes, app policy).
///
/// Emits `bstack` / `bstack_top` symbols; pass `bstack_top` to the
/// trampoline macro.
#[macro_export]
macro_rules! boot_stack {
    ($size:expr) => {
        core::arch::global_asm!(
            ".section .bss.bstack",
            ".global bstack",
            ".global bstack_top",
            "bstack:",
            concat!(".fill ", stringify!($size)),
            ".size bstack, . - bstack",
            "bstack_top:",
        );
    };
}

// ---------------------------------------------------------------------------
// riscv64
// ---------------------------------------------------------------------------

/// Emit the Sv39 boot trampoline: BSS clear, physical stack, call
/// kor-mmu's default `boot_mmu_build` hook, enable paging when the
/// returned offset is non-zero, cross the offset, and call `{entry}`.
///
/// # Safety
/// Expands to the raw entry assembly; call once per binary, from the app's
/// boot module, with `$stack_top` naming this app's boot-stack-top symbol.
#[macro_export]
macro_rules! boot_trampoline_riscv64 {
    ($stack_top:ident, $entry:ident) => {
        core::arch::global_asm!(
            r#"
.set SATP_MODE_SV39, (8 << 60)

.section .bss.boot_page_table,"aw",@nobits
.balign 4096
.global _boot_pt_buffer
_boot_pt_buffer:
    .space 4096 * 2

.section .bss.boot_mmu_offset,"aw",@nobits
.balign 8
_boot_mmu_offset:
    .space 8

.section .text.entry
.global _start
_start:
    mv      s0, a0                 # hart_id
    mv      s1, a1                 # DTB pointer

    # --- Clear BSS (la = physical while the PC is physical) ---
    la      t0, _sbss
    la      t1, _ebss
    bgeu    t0, t1, 2f
1:
    sd      zero, (t0)
    addi    t0, t0, 8
    bltu    t0, t1, 1b
2:
    la      sp, {stack}

    # --- Ask the app for its boot mapping ---
    la      a0, _boot_pt_buffer
    li      a1, 4096 * 2
    call    boot_mmu_build         # a0 = phys->virt offset (0 = unmapped)

    la      t2, _boot_mmu_offset
    sd      a0, (t2)
    beqz    a0, 5f

    la      t0, _boot_pt_buffer
    srli    t0, t0, 12
    li      t1, SATP_MODE_SV39
    or      t0, t0, t1
    csrw    satp, t0
    sfence.vma

    # --- Cross the offset to virtual PCs ---
    la      t0, 3f
    add     t0, t0, a0
    jr      t0
3:
    add     sp, sp, a0
    mv      a0, s0
    mv      a1, s1
    call    {entry}
    j       .
5:
    mv      a0, s0
    mv      a1, s1
    call    {entry}
    j       .

# --- Secondary hart entry (SBI hart_start) ---
.global _secondary_start
_secondary_start:
    mv      s0, a0                 # hart_id
    mv      s1, a1                 # stack top (post-offset address)

    la      t0, _boot_mmu_offset
    ld      t2, (t0)
    beqz    t2, 4f

    la      t0, _boot_pt_buffer
    srli    t0, t0, 12
    li      t1, SATP_MODE_SV39
    or      t0, t0, t1
    csrw    satp, t0
    sfence.vma

    la      t0, 4f
    add     t0, t0, t2
    jr      t0
4:
    mv      sp, s1
    mv      tp, s0
    mv      a0, s0
    call    rust_entry_secondary
    j       .
"#,
            stack = sym $stack_top,
            entry = sym $entry,
        );
    };
}

/// Emit the `rust_entry` wrapper for riscv64: records hart id and DTB
/// pointer through kor-arch, then tail-calls the app's `kernel_main`.
///
/// # Safety
/// `kernel_main` must exist in the invoking crate and never return.
#[macro_export]
macro_rules! boot_entry_riscv64 {
    () => {
        #[unsafe(no_mangle)]
        extern "C" fn rust_entry(hart_id: usize, dtb: usize) {
            kor_arch::riscv64::smp::set_cpu_id(hart_id);
            kor_arch::riscv64::mm::set_dtb_ptr(dtb);
            crate::kernel_main()
        }
    };
}

// ---------------------------------------------------------------------------
// aarch64
// ---------------------------------------------------------------------------

/// Emit the AArch64 boot trampoline: runtime load delta, BSS clear,
/// physical stack, kor-mmu's default `boot_mmu_build` hook, MAIR/TCR/TTBR
/// programming and MMU enable when the offset is non-zero, offset
/// crossing, `{entry}`.
///
/// # Safety
/// Call once per binary; `$stack_top` names the app's boot-stack-top
/// symbol (its link-time VA; the trampoline derives the physical address).
#[macro_export]
macro_rules! boot_trampoline_aarch64 {
    ($stack_top:ident, $entry:ident) => {
        core::arch::global_asm!(
            r#"
.section .bss.boot_page_table,"aw",%nobits
.balign 4096
.global _boot_pt_buffer
_boot_pt_buffer:
    .zero 8192

.section .bss.boot_mmu_offset,"aw",%nobits
.balign 8
_boot_mmu_offset:
    .zero 8

.section .text.entry
.global _start
_start:
    mov     x19, x0                 // DTB pointer (PA)
    mrs     x20, mpidr_el1          // CPU ID

    // --- Runtime load delta: x24 = link VA here - physical PC here ---
    adr     x0, 9f
    ldr     x1, =9f
    sub     x24, x1, x0
9:

    // --- Clear BSS ---
    ldr     x0, =_sbss
    sub     x0, x0, x24
    ldr     x1, =_ebss
    sub     x1, x1, x24
    cmp     x0, x1
    b.ge    2f
1:
    str     xzr, [x0], #8
    cmp     x0, x1
    b.lt     1b
2:
    ldr     x0, ={stack}
    sub     x0, x0, x24
    mov     sp, x0

    // --- Ask the app for its boot mapping ---
    ldr     x0, =_boot_pt_buffer
    sub     x0, x0, x24
    mov     x1, #8192
    ldr     x8, =boot_mmu_build
    sub     x8, x8, x24
    blr     x8                       // x0 = offset (0 = unmapped)
    mov     x23, x0

    ldr     x1, =_boot_mmu_offset
    sub     x1, x1, x24
    str     x23, [x1]
    cbz     x23, 5f

    // --- MAIR / TCR / TTBR + MMU enable (mechanism) ---
    movz    x0, #0xFF00
    movk    x0, #0x0044, lsl #16
    msr     mair_el1, x0
    ldr     x0, =0x0000000575103510
    msr     tcr_el1, x0
    isb
    ldr     x0, =_boot_pt_buffer
    sub     x0, x0, x24
    msr     ttbr0_el1, x0
    msr     ttbr1_el1, x0
    tlbi    vmalle1
    dsb     sy
    isb
    mrs     x0, sctlr_el1
    movz    x1, #0x1005
    orr     x0, x0, x1
    msr     sctlr_el1, x0
    isb

    // --- Cross the offset ---
    adr     x0, 3f
    add     x0, x0, x23
    br      x0
3:
    add     sp, sp, x23
    add     x19, x19, x23
    and     x0, x20, #0xffffff
    mov     x1, x19
    ldr     x8, ={entry}
    blr     x8
    b       .
5:
    and     x0, x20, #0xffffff
    mov     x1, x19
    ldr     x8, ={entry}
    sub     x8, x8, x24
    blr     x8
    b       .

// --- Secondary core entry (PSCI CPU_ON) ---
.global _secondary_start
_secondary_start:
    mov     x20, x0
    mrs     x19, mpidr_el1
    and     x19, x19, #0xffffff

    adr     x0, 9f
    ldr     x1, =9f
    sub     x24, x1, x0
9:
    ldr     x0, =_boot_mmu_offset
    sub     x0, x0, x24
    ldr     x23, [x0]
    cbz     x23, 5f

    movz    x0, #0xFF00
    movk    x0, #0x0044, lsl #16
    msr     mair_el1, x0
    ldr     x0, =0x0000000575103510
    msr     tcr_el1, x0
    isb
    ldr     x0, =_boot_pt_buffer
    sub     x0, x0, x24
    msr     ttbr0_el1, x0
    msr     ttbr1_el1, x0
    tlbi    vmalle1
    dsb     sy
    isb
    mrs     x0, sctlr_el1
    movz    x1, #0x1005
    orr     x0, x0, x1
    msr     sctlr_el1, x0
    isb

    adr     x0, 4f
    add     x0, x0, x23
    br      x0
4:
    mov     sp, x20
    mov     x0, x19
    ldr     x8, =rust_entry_secondary
    blr     x8
    b       .
5:
    mov     sp, x20
    mov     x0, x19
    ldr     x8, =rust_entry_secondary
    sub     x8, x8, x24
    blr     x8
    b       .
"#,
            stack = sym $stack_top,
            entry = sym $entry,
        );
    };
}

/// Emit the `rust_entry` wrapper for aarch64.
///
/// # Safety
/// `kernel_main` must exist in the invoking crate and never return.
#[macro_export]
macro_rules! boot_entry_aarch64 {
    () => {
        #[unsafe(no_mangle)]
        extern "C" fn rust_entry(_hart_id: usize, dtb: usize) {
            kor_arch::aarch64::mm::set_dtb_ptr(dtb);
            crate::kernel_main()
        }
    };
}

// ---------------------------------------------------------------------------
// x86_64 (Multiboot1; long mode requires the fixed identity tables)
// ---------------------------------------------------------------------------

/// Emit the Multiboot1 header + 32-bit stage + fixed boot tables + 64-bit
/// entry handoff.  Long mode requires paging, so the tables are built
/// inline with a fixed policy (identity low 4 GiB + high-half, 2 MiB
/// leaves); there is deliberately no `boot_mmu_build` call here.
///
/// # Safety
/// Call once per binary; `$stack_top` names the app's boot-stack-top
/// symbol (link-time VA); `$entry` receives (magic, mbi).
#[macro_export]
macro_rules! boot_trampoline_x86_64 {
    ($stack_top:ident, $entry:ident) => {
        core::arch::global_asm!(
            r#"
.macro Page2MTable base
    .set    n, \base
    .rept   512
        .quad n + 0x83
        .set    n, n + 0x200000
    .endr
.endm

.set MB_FLAGS, (1 << 0) | (1 << 6) | (1 << 16)
.set MB_MAGIC, 0x1BADB002

.section .multiboot
.balign 4
.type multiboot_header, STT_OBJECT
multiboot_header:
    .int    MB_MAGIC
    .int    MB_FLAGS
    .int    -(MB_MAGIC + MB_FLAGS)
    .int    multiboot_header - {kernel_offset}
    .int    _skernel - {kernel_offset}
    .int    _load_end - {kernel_offset}
    .int    _end - {kernel_offset}
    .int    _start - {kernel_offset}

.code32
.global _start
_start:
    mov     [.Lsaved_magic - {kernel_offset}], eax
    mov     [.Lsaved_mbi - {kernel_offset}], ebx

    lgdt    [.Ltmp_gdt_desc - {kernel_offset}]

    mov     ax, 0x18
    mov     ss, ax
    mov     ds, ax
    mov     es, ax
    mov     fs, ax
    mov     gs, ax

    # `mov esp, imm` in Intel syntax is a memory load; encode AT&T-style.
    mov     esp, {stack} - {kernel_offset}

    # enable PAE + PGE
    mov     eax, cr4
    or      eax, {cr4}
    mov     cr4, eax

    # load boot page table
    lea     eax, [_boot_page_table - {kernel_offset}]
    mov     cr3, eax

    # enable Long Mode in IA32_EFER
    mov     ecx, {efer_msr}
    rdmsr
    or      eax, {efer}
    wrmsr

    # enable Protected Mode + Paging
    mov     eax, cr0
    or      eax, {cr0}
    mov     cr0, eax

    ljmp    0x10, offset .Lentry64 - {kernel_offset}

.code64
.Lentry64:
    xor     ax, ax
    mov     ss, ax
    mov     ds, ax
    mov     es, ax
    mov     fs, ax
    mov     gs, ax

    # Relocate the stack to the high-half VA: the 32-bit stage set esp to
    # the physical address; high-half pointers are what Rust expects.
    movabs  rsp, offset {stack}

    mov     rdi, [.Lsaved_magic - {kernel_offset}]
    mov     rsi, [.Lsaved_mbi - {kernel_offset}]
    movabs  rax, offset {entry}
    call    rax

.Lhlt:
    hlt
    jmp     .Lhlt

.section .rodata
.balign 8
.Ltmp_gdt_desc:
    .short  .Ltmp_gdt_end - .Ltmp_gdt - 1
    .long   .Ltmp_gdt - {kernel_offset}

.section .data
.balign 8
.Lsaved_magic:
    .quad 0
.Lsaved_mbi:
    .quad 0

.balign 16
.Ltmp_gdt:
    .quad 0x0000000000000000    # null
    .quad 0x00cf9b000000ffff    # 32-bit code
    .quad 0x00af9b000000ffff    # 64-bit code
    .quad 0x00cf93000000ffff    # 32-bit data
.Ltmp_gdt_end:

.balign 4096
.global _boot_page_table
_boot_page_table:
    .quad _boot_mapping_pdpt - {kernel_offset} + 0x3
    .zero 8 * 255
    .quad _boot_mapping_pdpt - {kernel_offset} + 0x3
    .zero 8 * 255

.balign 4096
.global _boot_mapping_pdpt
_boot_mapping_pdpt:
    .quad .Lboot_pd1 - {kernel_offset} + 0x3
    .quad .Lboot_pd2 - {kernel_offset} + 0x3
    .quad .Lboot_pd3 - {kernel_offset} + 0x3
    .quad .Lboot_pd4 - {kernel_offset} + 0x3
    .zero 8 * 508

.balign 4096
.Lboot_pd1:
    Page2MTable 0x00000000
.balign 4096
.Lboot_pd2:
    Page2MTable 0x40000000
.balign 4096
.Lboot_pd3:
    Page2MTable 0x80000000
.balign 4096
.Lboot_pd4:
    Page2MTable 0xc0000000
"#,
            stack = sym $stack_top,
            entry = sym $entry,
            kernel_offset = const 0xffff_8000_0000_0000usize,
            cr0 = const 0x8001_0023u32,          // PG | PE | MP | NE | WP
            cr4 = const 0x6A0u32,                  // PAE | PGE | OSFXSR | OSXMMEXCPT
            efer_msr = const 0xC000_0080u32,
            efer = const 0x100u64,                // LME
        );
    };
}

/// Emit the `rust_entry` wrapper for x86_64 Multiboot.
///
/// # Safety
/// `kernel_main` must exist in the invoking crate and never return.
#[macro_export]
macro_rules! boot_entry_x86_64 {
    () => {
        #[unsafe(no_mangle)]
        extern "C" fn rust_entry(_magic: usize, mbi: usize) {
            kor_arch::x86_64::mm::set_multiboot_info(mbi);
            crate::kernel_main()
        }
    };
}
