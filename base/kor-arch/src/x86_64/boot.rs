//! x86_64 boot fragments (uniform names — see `crate::boot`).
//!
//! x86_64 has no runtime mapping slot: long mode requires paging before
//! the first instruction, so the mapping policy is table *data* — the
//! default tables are [`boot_pt_buffer_asm!`], and an application with a
//! different policy replaces that fragment (but must still define
//! `_boot_page_table`, a PML4 whose first and 256th entries point at a
//! PDPT).  The `Page2MTable` assembler macro used by the default tables is
//! defined by [`boot_setup_asm!`], so compose it *after* the setup
//! fragment.
//!
//! Composition order: `boot_stack_asm!` → `boot_setup_asm!` →
//! `boot_pt_buffer_asm!` (the policy) → `boot_cross_asm!` →
//! `boot_call_asm!(entry)`; `boot_secondary_asm!` is a no-op (AP bring-up
//! lives in `ap_boot.S`).
use crate::kernel_main;


/// Kernel virtual-base offset used by the physical-address stage and the
/// Multiboot header.  Shared by every koros-family link script (x86_64
/// high-half, offset `0xffff_8000_0000_0000`).
#[macro_export]
macro_rules! boot_setup_asm {
    () => {
        "
.macro Page2MTable base
    .set    n, \\base
    .rept   512
        .quad n + 0x83              # present|rw|ps|accessed leaf
        .set    n, n + 0x200000
    .endr
.endm

.set KOR_X86_KERNEL_OFFSET, 0xffff800000000000
.set KOR_X86_MB_MAGIC, 0x1BADB002
.set KOR_X86_MB_FLAGS, (1 << 0) | (1 << 6) | (1 << 16)

.section .multiboot
.balign 4
.type multiboot_header, STT_OBJECT
multiboot_header:
    .int    KOR_X86_MB_MAGIC
    .int    KOR_X86_MB_FLAGS
    .int    -(KOR_X86_MB_MAGIC + KOR_X86_MB_FLAGS)
    .int    multiboot_header - KOR_X86_KERNEL_OFFSET
    .int    _skernel - KOR_X86_KERNEL_OFFSET
    .int    _load_end - KOR_X86_KERNEL_OFFSET
    .int    _end - KOR_X86_KERNEL_OFFSET
    .int    _start - KOR_X86_KERNEL_OFFSET

.code32
.global _start
_start:
    mov     [.Lkor_x86_magic - KOR_X86_KERNEL_OFFSET], eax
    mov     [.Lkor_x86_mbi - KOR_X86_KERNEL_OFFSET], ebx

    lgdt    [.Lkor_x86_gdt_desc - KOR_X86_KERNEL_OFFSET]

    mov     ax, 0x18
    mov     ss, ax
    mov     ds, ax
    mov     es, ax
    mov     fs, ax
    mov     gs, ax

    # `mov esp, imm` in Intel syntax is a memory load; AT&T form
    mov     esp, bstack_top - KOR_X86_KERNEL_OFFSET

    mov     eax, cr4
    or      eax, 0x6A0               # PAE | PGE | OSFXSR | OSXMMEXCPT
    mov     cr4, eax

    lea     eax, [_boot_page_table - KOR_X86_KERNEL_OFFSET]
    mov     cr3, eax

    mov     ecx, 0xC0000080          # IA32_EFER
    rdmsr
    or      eax, 0x100               # LME
    wrmsr

    mov     eax, cr0
    or      eax, 0x80010023          # PG | PE | MP | NE | WP
    mov     cr0, eax

    ljmp    0x10, offset .Lkor_x86_entry64 - KOR_X86_KERNEL_OFFSET

.section .rodata
.balign 8
.Lkor_x86_gdt_desc:
    .short  .Lkor_x86_gdt_end - .Lkor_x86_gdt - 1
    .long   .Lkor_x86_gdt - KOR_X86_KERNEL_OFFSET

.section .data
.balign 8
.Lkor_x86_magic:
    .quad 0
.Lkor_x86_mbi:
    .quad 0

.balign 16
.Lkor_x86_gdt:
    .quad 0x0000000000000000    # null
    .quad 0x00cf9b000000ffff    # 32-bit code
    .quad 0x00af9b000000ffff    # 64-bit code
    .quad 0x00cf93000000ffff    # 32-bit data
.Lkor_x86_gdt_end:
"
    };
}

/// Default boot tables — the x86_64 *mapping policy*: identity low 4 GiB +
/// high-half window with 2 MiB leaves, at `_boot_page_table`.  Uses the
/// `Page2MTable` macro defined by [`boot_setup_asm!`], so compose this
/// fragment after it.
#[macro_export]
macro_rules! boot_pt_buffer_asm {
    () => {
        "
.section .data
.balign 4096
.global _boot_page_table
_boot_page_table:
    .quad _boot_mapping_pdpt - KOR_X86_KERNEL_OFFSET + 0x3
    .zero 8 * 255
    .quad _boot_mapping_pdpt - KOR_X86_KERNEL_OFFSET + 0x3
    .zero 8 * 255

.balign 4096
.global _boot_mapping_pdpt
_boot_mapping_pdpt:
    .quad .Lkor_x86_pd1 - KOR_X86_KERNEL_OFFSET + 0x3
    .quad .Lkor_x86_pd2 - KOR_X86_KERNEL_OFFSET + 0x3
    .quad .Lkor_x86_pd3 - KOR_X86_KERNEL_OFFSET + 0x3
    .quad .Lkor_x86_pd4 - KOR_X86_KERNEL_OFFSET + 0x3
    .zero 8 * 508

.balign 4096
.Lkor_x86_pd1:
    Page2MTable 0x00000000
.balign 4096
.Lkor_x86_pd2:
    Page2MTable 0x40000000
.balign 4096
.Lkor_x86_pd3:
    Page2MTable 0x80000000
.balign 4096
.Lkor_x86_pd4:
    Page2MTable 0xc0000000
"
    };
}

/// The 64-bit stage (`PC` target of the 32-bit stage's long jump): drop the
/// temporary segments and relocate the stack to the high-half VA.
#[macro_export]
macro_rules! boot_cross_asm {
    () => {
        "
.code64
.Lkor_x86_entry64:
    xor     ax, ax
    mov     ss, ax
    mov     ds, ax
    mov     es, ax
    mov     fs, ax
    mov     gs, ax

    # the 32-bit stage set esp to the physical address; Rust
    # expects the high-half pointer
    movabs  rsp, offset bstack_top
"
    };
}

/// Restore the Multiboot registers and call the application entry
/// (receives `(magic, mbi)`); parks if it ever returns.
#[macro_export]
macro_rules! boot_call_asm {
    ($entry:ident) => {
        concat!(
            "
    mov     rdi, [.Lkor_x86_magic - KOR_X86_KERNEL_OFFSET]
    mov     rsi, [.Lkor_x86_mbi - KOR_X86_KERNEL_OFFSET]
    movabs  rax, offset ",
            stringify!($entry),
            "
    call    rax

.Lkor_x86_hlt:
    hlt
    jmp     .Lkor_x86_hlt
",
        )
    };
}

/// No-op on x86_64: secondary-CPU bring-up is the real-mode INIT-SIPI-SIPI
/// trampoline in `ap_boot.S`, which is not fragment-composable.
#[macro_export]
macro_rules! boot_secondary_asm {
    () => { "" };
}

/// First Rust function of the boot flow (`call rust_entry` from the app's
/// composed trampoline): stash the Multiboot info pointer into kor-arch
/// state, then enter the application's `kernel_main` (link-time extern).
#[unsafe(no_mangle)]
extern "C" fn rust_entry(_magic: usize, mbi: usize) {
    super::mm::set_multiboot_info(mbi);
    // SAFETY: the app binary provides kernel_main at link time; it never
    // returns.
    unsafe { kernel_main() }
}
