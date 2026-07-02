//! x86_64 trap handling.
//!
//! Builds an IDT with 256 entries (each pointing to an assembly stub that
//! saves registers), loads the IDT, and prints trap info on exceptions.

use crate::println;
use core::arch::global_asm;

// ---------------------------------------------------------------------------
// TrapFrame — matches the stack layout after the assembly stub pushes regs
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct TrapFrame {
    // Pushed by .Ltrap_common (in order: rax … r15)
    rax: u64,
    rcx: u64,
    rdx: u64,
    rbx: u64,
    rbp: u64,
    rsi: u64,
    rdi: u64,
    r8:  u64,
    r9:  u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    // Pushed by each stub
    vector:     u64,
    error_code: u64,
    // Pushed by CPU
    rip:    u64,
    cs:     u64,
    rflags: u64,
}

// ---------------------------------------------------------------------------
// Assembly stubs and handler table
// ---------------------------------------------------------------------------

global_asm!(include_str!("trap.S"));

unsafe extern "C" {
    #[allow(dead_code)]
    static trap_handler_table: [*const (); 256];
}

// ---------------------------------------------------------------------------
// Rust trap handler — called from assembly
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
extern "C" fn handle_trap(tf: &mut TrapFrame) {
    let description = match tf.vector {
        0  => "Divide-by-zero",
        1  => "Debug",
        2  => "Non-maskable Interrupt",
        3  => "Breakpoint",
        4  => "Overflow",
        5  => "Bound Range Exceeded",
        6  => "Invalid Opcode",
        7  => "Device Not Available",
        8  => "Double Fault",
        9  => "Coprocessor Segment Overrun",
        10 => "Invalid TSS",
        11 => "Segment Not Present",
        12 => "Stack-Segment Fault",
        13 => "General Protection Fault",
        14 => "Page Fault",
        16 => "x87 Floating-Point Exception",
        17 => "Alignment Check",
        18 => "Machine Check",
        19 => "SIMD Floating-Point Exception",
        20 => "Virtualization Exception",
        30 => "Security Exception",
        v if v < 256 => "User-defined Interrupt",
        _ => "Unknown",
    };

    println!("");
    println!("=== TRAP ===");
    println!("Vector:     {} ({})", tf.vector, description);
    println!("Error code: {:#x}", tf.error_code);
    println!("RIP:        {:#018x}", tf.rip);
    println!("RFLAGS:     {:#018x}", tf.rflags);
    println!("");

    println!("Register dump:");
    println!("  rax={:#018x} rcx={:#018x} rdx={:#018x} rbx={:#018x}", tf.rax, tf.rcx, tf.rdx, tf.rbx);
    println!("  rbp={:#018x} rsi={:#018x} rdi={:#018x} r8={:#018x}",  tf.rbp, tf.rsi, tf.rdi, tf.r8);
    println!("  r9={:#018x}  r10={:#018x} r11={:#018x} r12={:#018x}", tf.r9, tf.r10, tf.r11, tf.r12);
    println!("  r13={:#018x} r14={:#018x} r15={:#018x}", tf.r13, tf.r14, tf.r15);
    println!("  cs={:#018x}  ss={:#018x}  rsp={:#018x}",
        tf.cs, 0u64, 0u64);  // SS/RSP not saved for kernel-level traps

    panic!("Unhandled trap");
}

// ---------------------------------------------------------------------------
// Public init — build IDT, load it
// ---------------------------------------------------------------------------

/// Install the trap handler: build + load IDT.
pub fn init() {
    unsafe {
        _init_idt();
    }
}

unsafe extern "C" {
    fn _init_idt();
}
