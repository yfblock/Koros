# x86_64 application-processor (AP) boot trampoline.
#
# APs start in 16-bit real mode at CS:IP = (vector<<8):0 in response to a
# STARTUP IPI, so this blob is copied to a page-aligned physical address below
# 1 MiB (AP_BOOT_PADDR) and the SIPI vector is that page number.
#
# It brings the AP from real mode -> protected mode -> long mode using the
# kernel's existing page tables (CR3 supplied in a fixed slot), then jumps to
# the 64-bit Rust entry with the per-CPU stack.
#
# All internal references use `label - ap_start + AP_BOOT_PADDR` so the copied
# blob is position-correct at its runtime physical address.

.set AP_BOOT_PADDR, 0x8000

.set pa_ap32,      .Lap32 - ap_start + AP_BOOT_PADDR
.set pa_ap64,      .Lap64 - ap_start + AP_BOOT_PADDR
.set pa_gdt,       .Lgdt - ap_start + AP_BOOT_PADDR
.set pa_gdt_desc,  .Lgdt_desc - ap_start + AP_BOOT_PADDR

# Fixed data slots filled in by the boot CPU before each SIPI.
.set slot_cr3,     AP_BOOT_PADDR + 0xfe0
.set slot_stack,   AP_BOOT_PADDR + 0xff0
.set slot_entry,   AP_BOOT_PADDR + 0xff8

.section .text
.intel_syntax noprefix
.code16
.p2align 12
.global ap_start
ap_start:
    cli
    cld
    xor     ax, ax
    mov     ds, ax
    mov     es, ax
    mov     ss, ax
    mov     fs, ax
    mov     gs, ax

    lgdt    [pa_gdt_desc]

    mov     eax, cr0
    or      eax, 1                  # CR0.PE
    mov     cr0, eax
    ljmp    0x08, offset pa_ap32    # -> 32-bit code segment

.code32
.Lap32:
    mov     ax, 0x18                # data segment
    mov     ds, ax
    mov     es, ax
    mov     ss, ax

    mov     eax, cr4
    or      eax, (1 << 5)           # CR4.PAE
    mov     cr4, eax

    mov     eax, [slot_cr3]         # kernel PML4 physical base
    mov     cr3, eax

    mov     ecx, 0xC0000080         # IA32_EFER
    rdmsr
    or      eax, (1 << 8)           # EFER.LME
    wrmsr

    mov     eax, cr0
    or      eax, 0x80000001         # CR0.PG | CR0.PE
    mov     cr0, eax
    ljmp    0x10, offset pa_ap64    # -> 64-bit code segment

.code64
.Lap64:
    mov     rsp, [slot_stack]       # per-CPU stack top (VA, high-half mapped)
    mov     rax, [slot_entry]       # 64-bit Rust entry (VA)
    jmp     rax

.p2align 4
.Lgdt:
    .quad   0x0000000000000000      # null
    .quad   0x00cf9b000000ffff      # 0x08 32-bit code
    .quad   0x00af9b000000ffff      # 0x10 64-bit code
    .quad   0x00cf93000000ffff      # 0x18 data
.Lgdt_desc:
    .short  .Lgdt_desc - .Lgdt - 1
    .long   pa_gdt
.Lgdt_end:

.global ap_end
ap_end:
