//! Minimal host IDT: every vector routes to a single stub that prints a
//! marker and halts.  The hypervisor runs with interrupts disabled, so this
//! exists only to catch host bugs (e.g. a misconfigured VMRUN) instead of
//! triple-faulting silently.

use core::arch::{asm, global_asm};

global_asm!(
    ".intel_syntax noprefix",
    ".section .text",
    ".code64",
    ".global host_fault",
    "host_fault:",
    "    push rax",
    "    push rcx",
    "    push rdx",
    "    push rsi",
    "    push rdi",
    "    push r8",
    "    push r9",
    "    push r10",
    "    push r11",
    "    call {handler}",
    "    cli",
    "1:  hlt",
    "    jmp 1b",
    handler = sym host_fault_handler,
);

#[unsafe(no_mangle)]
extern "C" fn host_fault_handler() {
    kor::println!("");
    kor::println!("!!! HOST FAULT (CPU exception in hypervisor) !!!");
}

/// IDT: 256 16-byte entries, all pointing to `host_fault`.
#[repr(C, align(16))]
struct Idt([u8; 256 * 16]);

static mut IDT: Idt = Idt([0; 256 * 16]);

unsafe extern "C" {
    fn host_fault();
}

pub fn init() {
    unsafe {
        let base = host_fault as usize;
        let p = core::ptr::addr_of_mut!(IDT).cast::<u8>();
        for i in 0..256 {
            let off = i * 16;
            p.add(off).cast::<u16>().write_unaligned(base as u16);
            p.add(off + 2).cast::<u16>().write_unaligned(0x10); // 64-bit code sel
            p.add(off + 4).write_unaligned(0);                  // IST
            p.add(off + 5).write_unaligned(0x8E);                // present | int gate
            p.add(off + 6).cast::<u16>().write_unaligned((base >> 16) as u16);
            p.add(off + 8).cast::<u32>().write_unaligned((base >> 32) as u32);
        }
        // Build a 10-byte IDTR (limit, base) on the stack and load it.
        asm!(
            "sub rsp, 16",
            "mov word ptr [rsp], {limit}",
            "lea rax, [rip + {idt}]",
            "mov [rsp + 2], rax",
            "lidt [rsp]",
            "add rsp, 16",
            idt = sym IDT,
            limit = const 256 * 16 - 1,
            out("rax") _,
        );
    }
}
