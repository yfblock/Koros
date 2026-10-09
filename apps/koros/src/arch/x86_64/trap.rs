//! x86_64 trap handling for koros: IDT stubs composed from the kor-arch
//! fragment, dispatch implemented here.  koros has no user mode: the
//! timer vector drives the tick, everything else is fatal (the syscall
//! contracts exist only to satisfy the composed assembly).

use core::arch::global_asm;

use kor::println;
use kor_arch::TrapFrame;

global_asm!(kor_arch::trap_kernel_vec_asm!());

unsafe extern "C" {
    fn syscall_entry();
}

/// Native-syscall contract — unreachable without user mode.
#[unsafe(no_mangle)]
extern "C" fn handle_syscall(_tf: &mut TrapFrame) -> i32 {
    0
}

/// Application dispatch contract called by the IDT stubs.
#[unsafe(no_mangle)]
extern "C" fn trap_dispatch(tf: &mut TrapFrame) {
    // LAPIC timer vector (acknowledged by the stub): tick and preempt.
    if tf.vector == 0x20 {
        kor_arch::increment_tick();
        kor_arch::handle_tick();
        if kor_sched::is_ready() {
            kor_sched::timer_tick();
            kor_sched::preempt();
        }
        return;
    }

    report_and_panic(tf);
}

/// Print the frame and panic (shared fatal path).
pub fn report_and_panic(tf: &TrapFrame) -> ! {
    let description = match tf.vector {
        0 => "Divide-by-zero",
        1 => "Debug",
        2 => "Non-maskable Interrupt",
        3 => "Breakpoint",
        4 => "Overflow",
        5 => "Bound Range Exceeded",
        6 => "Invalid Opcode",
        7 => "Device Not Available",
        8 => "Double Fault",
        9 => "Coprocessor Segment Overrun",
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
    println!();
    println!("Register dump:");
    println!(
        "  rax={:#018x} rcx={:#018x} rdx={:#018x} rbx={:#018x}",
        tf.rax, tf.rcx, tf.rdx, tf.rbx
    );
    println!(
        "  rbp={:#018x} rsi={:#018x} rdi={:#018x} r8={:#018x}",
        tf.rbp, tf.rsi, tf.rdi, tf.r8
    );
    println!(
        "  r9={:#018x}  r10={:#018x} r11={:#018x} r12={:#018x}",
        tf.r9, tf.r10, tf.r11, tf.r12
    );
    println!(
        "  r13={:#018x} r14={:#018x} r15={:#018x}",
        tf.r13, tf.r14, tf.r15
    );
    println!(
        "  cs={:#018x}  ss={:#018x}  rsp={:#018x}",
        tf.cs, tf.ss, tf.rsp
    );
    panic!("Unhandled trap");
}

/// Install the composed vectors (GDT/TSS/IDT + native SYSCALL MSRs).
pub fn init() {
    kor_arch::x86_64::trap::install_vectors(syscall_entry as *const () as usize);
    println!("trap: GDT (user segments) + TSS + IDT loaded");
}
