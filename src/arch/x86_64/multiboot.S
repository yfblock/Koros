# x86_64 Multiboot boot — 32-bit entry to long mode, then call rust_entry.

.macro Page2MTable base
    .set    n, \base
    .rept   512
        .quad n + 0x83
        .set    n, n + 0x200000
    .endr
.endm

.macro MapAll2MPage name, addr
\name:
    Page2MTable  \addr
.endm

# Flags: bit 1 (memory info) + bit 16 (address fields).
.set MB_FLAGS, (1 << 1) | (1 << 16)

.section .multiboot
.balign 4
.type multiboot_header, STT_OBJECT
multiboot_header:
    .int    {mb_hdr_magic}                      # magic
    .int    {mb_hdr_flags}                      # flags
    .int    -({mb_hdr_magic} + {mb_hdr_flags})  # checksum
    .int    multiboot_header - {kernel_offset}   # header_addr
    .int    _skernel - {kernel_offset}           # load_addr
    .int    _load_end - {kernel_offset}          # load_end_addr
    .int    _end - {kernel_offset}               # bss_end_addr
    .int    _start - {kernel_offset}             # entry_addr

.code32
.global _start
_start:
    mov     edi, eax                            # save multiboot magic
    mov     esi, ebx                            # save multiboot info ptr

    lgdt    [.Ltmp_gdt_desc - {kernel_offset}]

    # set data segment selectors
    mov     ax, 0x18
    mov     ss, ax
    mov     ds, ax
    mov     es, ax
    mov     fs, ax
    mov     gs, ax

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
    # clear segment selectors
    xor     ax, ax
    mov     ss, ax
    mov     ds, ax
    mov     es, ax
    mov     fs, ax
    mov     gs, ax

    # --- Clear BSS ---
    movabs  rcx, offset _sbss
    movabs  rdx, offset _ebss
    cmp     rcx, rdx
    jae     .Lbss_done
.Lbss_loop:
    mov     byte ptr [rcx], 0
    inc     rcx
    cmp     rcx, rdx
    jb      .Lbss_loop
.Lbss_done:

    # set stack and call Rust entry
    movabs  rsp, offset bstack_top
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
MapAll2MPage  .Lboot_pd1, 0x00000000
MapAll2MPage  .Lboot_pd2, 0x40000000
MapAll2MPage  .Lboot_pd3, 0x80000000
MapAll2MPage  .Lboot_pd4, 0xc0000000
