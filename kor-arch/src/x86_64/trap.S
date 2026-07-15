# x86_64 trap entry.
#
# 256 handler stubs, each pushing a vector number and jumping to
# .Ltrap_common, which saves all GPRs and calls handle_trap.
#
# Also provides _init_idt: builds the IDT from the handler table
# in assembly and loads it.
#
# Uses Intel syntax because .altmacro mode conflicts with AT&T %prefix.

.equ NUM_INT, 256

.altmacro

.intel_syntax noprefix

.macro DEF_HANDLER i
.global trap_entry_\i
trap_entry_\i:
.if \i == 8 || (\i >= 10 && \i <= 14) || \i == 17
    push    \i                    # CPU already pushed error code
    jmp     .Ltrap_common
.else
    push    0                     # dummy error code
    push    \i                    # push vector
    jmp     .Ltrap_common
.endif
.endm

.macro DEF_TABLE_ENTRY i
    .quad trap_entry_\i - trap_handler_table
.endm

# -----------------------------------------------------------------------
# Handler stubs (in .text so they are executable)
# -----------------------------------------------------------------------
.section .text
.code64
.global _trap_handlers
_trap_handlers:
.set i, 0
.rept NUM_INT
    DEF_HANDLER %i
    .set i, i + 1
.endr


.section .text
.code64
.global syscall_entry
syscall_entry:
    # IA32_KERNEL_GS_BASE points at this CPU's SyscallCpuState. swapgs gives
    # access to it without clobbering any user register before the save frame.
    swapgs
    mov     qword ptr gs:[0], rsp
    mov     rsp, qword ptr gs:[8]

    # Build the same CPU frame as a ring-3 interrupt, followed by a synthetic
    # error code and vector 0x100. The Rust handler distinguishes this native
    # SYSCALL path from the int 0x80 compatibility gate.
    push    0x23
    push    qword ptr gs:[0]
    push    r11
    push    0x1b
    push    rcx
    push    0
    push    0x100

    push    r15
    push    r14
    push    r13
    push    r12
    push    r11
    push    r10
    push    r9
    push    r8
    push    rdi
    push    rsi
    push    rbp
    push    rbx
    push    rdx
    push    rcx
    push    rax

    mov     rdi, rsp
    call    handle_trap

    pop     rax
    pop     rcx
    pop     rdx
    pop     rbx
    pop     rbp
    pop     rsi
    pop     rdi
    pop     r8
    pop     r9
    pop     r10
    pop     r11
    pop     r12
    pop     r13
    pop     r14
    pop     r15
    add     rsp, 16
    swapgs
    iretq

.Ltrap_common:
    # 64-bit long mode: the CPU ALWAYS pushes SS:RSP:RFLAGS:CS:RIP (5 qwords)
    # on every interrupt/exception, regardless of a privilege-level change
    # (unlike 32-bit mode, which omits SS:RSP when CPL is unchanged).  The
    # stub already pushed error_code + vector, so the frame below the GPRs is
    #   [vector][error_code][rip][cs][rflags][rsp][ss]  — uniform for kernel
    # and user traps.  No conditional SS:RSP padding is needed; `iretq` always
    # pops all five.
    push    r15
    push    r14
    push    r13
    push    r12
    push    r11
    push    r10
    push    r9
    push    r8
    push    rdi
    push    rsi
    push    rbp
    push    rbx
    push    rdx
    push    rcx
    push    rax

    mov     rdi, rsp
    call    handle_trap

    pop     rax
    pop     rcx
    pop     rdx
    pop     rbx
    pop     rbp
    pop     rsi
    pop     rdi
    pop     r8
    pop     r9
    pop     r10
    pop     r11
    pop     r12
    pop     r13
    pop     r14
    pop     r15

    add     rsp, 16

    iretq

# -----------------------------------------------------------------------
# Handler table for IDT construction (same section as stubs!)
# -----------------------------------------------------------------------
.section .text
.global trap_handler_table
trap_handler_table:
.set i, 0
.rept NUM_INT
    DEF_TABLE_ENTRY %i
    .set i, i + 1
.endr

# -----------------------------------------------------------------------
# Static IDT in .data (readable, not executable)
# -----------------------------------------------------------------------
.section .data
.balign 16
.global _test_idt
_test_idt:
    .skip 4096, 0

# -----------------------------------------------------------------------
# _init_idt: build all 256 IDT entries from handler table, load IDT, return.
# -----------------------------------------------------------------------
.section .text
.code64
.global _init_idt
_init_idt:
    push    r12
    push    r13
    push    r14
    # r12 = IDT base address
    lea     r12, _test_idt[rip]
    # r13 = trap_handler_table
    lea     r13, trap_handler_table[rip]
    # r14 = byte offset into IDT (0, 16, 32, ...)
    xor     r14, r14

.Lbuild_loop:
    # handler = table[i] where i = r14 >> 4, offset = (r14 >> 4) * 8 = r14 >> 1
    mov     rcx, r14
    shr     rcx, 1
    mov     rax, [r13 + rcx]    # load PC-relative offset
    add     rax, r13            # absolute address = table_base + offset

    # Build IDT entry at r12 + r14
    mov     [r12 + r14], ax                  # offset_low (bits 0-15)
    mov     word ptr [r12 + r14 + 2], 0x08   # selector: kcode64 (0x08) — handler CS
    mov     byte ptr [r12 + r14 + 4], 0      # IST
    mov     byte ptr [r12 + r14 + 5], 0x8E   # flags: present, DPL=0, interrupt gate
    # int 0x80 (syscall gate) must be reachable from ring 3: DPL=3.
    cmp     r14, 0x80 * 16
    jne     .Lno_dpl3
    mov     byte ptr [r12 + r14 + 5], 0xEE
.Lno_dpl3:
    shr     rax, 16
    mov     [r12 + r14 + 6], ax              # offset_mid (bits 16-31)
    shr     rax, 16
    mov     [r12 + r14 + 8], eax             # offset_high (bits 32-63)
    # bytes 12-15 remain zero (from .skip 4096,0)

    add     r14, 16
    cmp     r14, 4096
    jne     .Lbuild_loop

    # Build IDTR on stack, load it
    # NB: .intel_syntax – bare symbol = memory reference! Use lea for address.
    sub     rsp, 16
    mov     word ptr [rsp], 4095             # limit = 256*16 - 1
    mov     [rsp + 2], r12                   # base = _test_idt address
    lidt    [rsp]
    add     rsp, 16

    pop     r14
    pop     r13
    pop     r12
    ret

# -----------------------------------------------------------------------
# _load_gdt: load the GDT (built in Rust), reload all segment registers to
# the new layout (kcode64=0x08, kdata=0x10, FS/GS=null), and load the task
# register (TSS selector 0x28).  rdi = pointer to a 10-byte GDTR
# (limit:16, base:64).  Interrupts must be disabled.
# -----------------------------------------------------------------------
.section .text
.code64
.global _load_gdt
_load_gdt:
    lgdt    [rdi]

    # Reload data segments to kdata (0x10); clear FS/GS.
    mov     ax, 0x10
    mov     ds, ax
    mov     es, ax
    mov     ss, ax
    xor     eax, eax
    mov     fs, ax
    mov     gs, ax

    # Reload CS to kcode64 (0x08) via a far return.  The boot GDT placed
    # 64-bit code at selector 0x10; the new GDT places it at 0x08, so CS must
    # be reloaded before any interrupt fires (IDT handlers run with CS=0x08).
    push    0x08
    lea     rax, [rip + .Lgdt_cs_ok]
    push    rax
    retfq
.Lgdt_cs_ok:

    # Load the task register with the TSS selector.
    mov     ax, 0x28
    ltr     ax
    ret
