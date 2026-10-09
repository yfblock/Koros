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

/// Kernel virtual-base offset used by the physical-address stage and the
/// Multiboot header.  Shared by every koros-family link script (x86_64
/// high-half, offset `0xffff_8000_0000_0000`).
#[macro_export]
macro_rules! boot_setup_asm {
    () => {
        concat!(
            ".macro Page2MTable base\n",
            "    .set    n, \\base\n",
            "    .rept   512\n",
            "        .quad n + 0x83              # present|rw|ps|accessed leaf\n",
            "        .set    n, n + 0x200000\n",
            "    .endr\n",
            ".endm\n",
            "\n",
            ".set KOR_X86_KERNEL_OFFSET, 0xffff800000000000\n",
            ".set KOR_X86_MB_MAGIC, 0x1BADB002\n",
            ".set KOR_X86_MB_FLAGS, (1 << 0) | (1 << 6) | (1 << 16)\n",
            "\n",
            ".section .multiboot\n",
            ".balign 4\n",
            ".type multiboot_header, STT_OBJECT\n",
            "multiboot_header:\n",
            "    .int    KOR_X86_MB_MAGIC\n",
            "    .int    KOR_X86_MB_FLAGS\n",
            "    .int    -(KOR_X86_MB_MAGIC + KOR_X86_MB_FLAGS)\n",
            "    .int    multiboot_header - KOR_X86_KERNEL_OFFSET\n",
            "    .int    _skernel - KOR_X86_KERNEL_OFFSET\n",
            "    .int    _load_end - KOR_X86_KERNEL_OFFSET\n",
            "    .int    _end - KOR_X86_KERNEL_OFFSET\n",
            "    .int    _start - KOR_X86_KERNEL_OFFSET\n",
            "\n",
            ".code32\n",
            ".global _start\n",
            "_start:\n",
            "    mov     [.Lkor_x86_magic - KOR_X86_KERNEL_OFFSET], eax\n",
            "    mov     [.Lkor_x86_mbi - KOR_X86_KERNEL_OFFSET], ebx\n",
            "\n",
            "    lgdt    [.Lkor_x86_gdt_desc - KOR_X86_KERNEL_OFFSET]\n",
            "\n",
            "    mov     ax, 0x18\n",
            "    mov     ss, ax\n",
            "    mov     ds, ax\n",
            "    mov     es, ax\n",
            "    mov     fs, ax\n",
            "    mov     gs, ax\n",
            "\n",
            "    # `mov esp, imm` in Intel syntax is a memory load; AT&T form\n",
            "    mov     esp, bstack_top - KOR_X86_KERNEL_OFFSET\n",
            "\n",
            "    mov     eax, cr4\n",
            "    or      eax, 0x6A0               # PAE | PGE | OSFXSR | OSXMMEXCPT\n",
            "    mov     cr4, eax\n",
            "\n",
            "    lea     eax, [_boot_page_table - KOR_X86_KERNEL_OFFSET]\n",
            "    mov     cr3, eax\n",
            "\n",
            "    mov     ecx, 0xC0000080          # IA32_EFER\n",
            "    rdmsr\n",
            "    or      eax, 0x100               # LME\n",
            "    wrmsr\n",
            "\n",
            "    mov     eax, cr0\n",
            "    or      eax, 0x80010023          # PG | PE | MP | NE | WP\n",
            "    mov     cr0, eax\n",
            "\n",
            "    ljmp    0x10, offset .Lkor_x86_entry64 - KOR_X86_KERNEL_OFFSET\n",
            "\n",
            ".section .rodata\n",
            ".balign 8\n",
            ".Lkor_x86_gdt_desc:\n",
            "    .short  .Lkor_x86_gdt_end - .Lkor_x86_gdt - 1\n",
            "    .long   .Lkor_x86_gdt - KOR_X86_KERNEL_OFFSET\n",
            "\n",
            ".section .data\n",
            ".balign 8\n",
            ".Lkor_x86_magic:\n",
            "    .quad 0\n",
            ".Lkor_x86_mbi:\n",
            "    .quad 0\n",
            "\n",
            ".balign 16\n",
            ".Lkor_x86_gdt:\n",
            "    .quad 0x0000000000000000    # null\n",
            "    .quad 0x00cf9b000000ffff    # 32-bit code\n",
            "    .quad 0x00af9b000000ffff    # 64-bit code\n",
            "    .quad 0x00cf93000000ffff    # 32-bit data\n",
            ".Lkor_x86_gdt_end:\n",
        )
    };
}

/// Default boot tables — the x86_64 *mapping policy*: identity low 4 GiB +
/// high-half window with 2 MiB leaves, at `_boot_page_table`.  Uses the
/// `Page2MTable` macro defined by [`boot_setup_asm!`], so compose this
/// fragment after it.
#[macro_export]
macro_rules! boot_pt_buffer_asm {
    () => {
        concat!(
            ".section .data\n",
            ".balign 4096\n",
            ".global _boot_page_table\n",
            "_boot_page_table:\n",
            "    .quad _boot_mapping_pdpt - KOR_X86_KERNEL_OFFSET + 0x3\n",
            "    .zero 8 * 255\n",
            "    .quad _boot_mapping_pdpt - KOR_X86_KERNEL_OFFSET + 0x3\n",
            "    .zero 8 * 255\n",
            "\n",
            ".balign 4096\n",
            ".global _boot_mapping_pdpt\n",
            "_boot_mapping_pdpt:\n",
            "    .quad .Lkor_x86_pd1 - KOR_X86_KERNEL_OFFSET + 0x3\n",
            "    .quad .Lkor_x86_pd2 - KOR_X86_KERNEL_OFFSET + 0x3\n",
            "    .quad .Lkor_x86_pd3 - KOR_X86_KERNEL_OFFSET + 0x3\n",
            "    .quad .Lkor_x86_pd4 - KOR_X86_KERNEL_OFFSET + 0x3\n",
            "    .zero 8 * 508\n",
            "\n",
            ".balign 4096\n",
            ".Lkor_x86_pd1:\n",
            "    Page2MTable 0x00000000\n",
            ".balign 4096\n",
            ".Lkor_x86_pd2:\n",
            "    Page2MTable 0x40000000\n",
            ".balign 4096\n",
            ".Lkor_x86_pd3:\n",
            "    Page2MTable 0x80000000\n",
            ".balign 4096\n",
            ".Lkor_x86_pd4:\n",
            "    Page2MTable 0xc0000000\n",
        )
    };
}

/// The 64-bit stage (`PC` target of the 32-bit stage's long jump): drop the
/// temporary segments and relocate the stack to the high-half VA.
#[macro_export]
macro_rules! boot_cross_asm {
    () => {
        concat!(
            ".code64\n",
            ".Lkor_x86_entry64:\n",
            "    xor     ax, ax\n",
            "    mov     ss, ax\n",
            "    mov     ds, ax\n",
            "    mov     es, ax\n",
            "    mov     fs, ax\n",
            "    mov     gs, ax\n",
            "\n",
            "    # the 32-bit stage set esp to the physical address; Rust\n",
            "    # expects the high-half pointer\n",
            "    movabs  rsp, offset bstack_top\n",
        )
    };
}

/// Restore the Multiboot registers and call the application entry
/// (receives `(magic, mbi)`); parks if it ever returns.
#[macro_export]
macro_rules! boot_call_asm {
    ($entry:ident) => {
        concat!(
            "    mov     rdi, [.Lkor_x86_magic - KOR_X86_KERNEL_OFFSET]\n",
            "    mov     rsi, [.Lkor_x86_mbi - KOR_X86_KERNEL_OFFSET]\n",
            "    movabs  rax, offset ",
            stringify!($entry),
            "\n",
            "    call    rax\n",
            "\n",
            ".Lkor_x86_hlt:\n",
            "    hlt\n",
            "    jmp     .Lkor_x86_hlt\n",
        )
    };
}

/// No-op on x86_64: secondary-CPU bring-up is the real-mode INIT-SIPI-SIPI
/// trampoline in `ap_boot.S`, which is not fragment-composable.
#[macro_export]
macro_rules! boot_secondary_asm {
    () => { "" };
}

/// Emit the `rust_entry` wrapper: records the Multiboot info pointer
/// through kor-arch, then tail-calls the app's `kernel_main`.
///
/// # Safety
/// `kernel_main` must exist in the invoking crate and never return.
#[macro_export]
macro_rules! boot_entry {
    () => {
        #[unsafe(no_mangle)]
        extern "C" fn rust_entry(_magic: usize, mbi: usize) {
            kor_arch::x86_64::mm::set_multiboot_info(mbi);
            crate::kernel_main()
        }
    };
}
