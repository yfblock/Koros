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

# Flags: bit 0 (mem info) + bit 6 (mmap) + bit 16 (address fields).
.set MB_FLAGS, (1 << 0) | (1 << 6) | (1 << 16)

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
    mov     [.Lsaved_magic - {kernel_offset}], eax
    mov     [.Lsaved_mbi - {kernel_offset}], ebx

    lgdt    [.Ltmp_gdt_desc - {kernel_offset}]

    # set data segment selectors
    mov     ax, 0x18
    mov     ss, ax
    mov     ds, ax
    mov     es, ax
    mov     fs, ax
    mov     gs, ax

    # set stack (physical address, 32-bit mode)
    # AT&T syntax for immediate load: .intel_syntax treats "mov esp, expr" as memory load!
    .att_syntax
    movl    $bstack_top - {kernel_offset}, %esp
    .intel_syntax noprefix

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

    # Relocate the stack to the high-half virtual address.
    # During the 32-bit stage, esp was set to the *physical* address of
    # bstack_top (bstack_top - KERNEL_OFFSET).  The identity map (PML4[0])
    # makes low addresses work for plain loads/stores, but `virt_to_phys`
    # in Rust expects all pointers to live in the high-half direct map.
    # Without this fix, DMA addresses derived from stack variables (e.g.
    # virtio-blk request headers) would underflow and produce garbage.
    movabs  rsp, offset bstack_top

    mov     rdi, [.Lsaved_magic - {kernel_offset}]
    mov     rsi, [.Lsaved_mbi - {kernel_offset}]

    # call Rust entry (System V ABI: rdi, rsi)
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
MapAll2MPage  .Lboot_pd1, 0x00000000
MapAll2MPage  .Lboot_pd2, 0x40000000
MapAll2MPage  .Lboot_pd3, 0x80000000
MapAll2MPage  .Lboot_pd4, 0xc0000000
