//! x86_64 trap handling for korix: IDT stubs composed from the kor-arch
//! fragment, dispatch implemented here — the timer vector (kernel mode
//! only; user mode runs unpreempted), both syscall entries (native SYSCALL
//! and `int 0x80`), the exit vector, and user page-fault reporting.

use core::arch::global_asm;

use kor::println;
use kor_arch::TrapFrame;

global_asm!(kor_arch::trap_kernel_vec_asm!());

unsafe extern "C" {
    fn syscall_entry();
}

/// Native-syscall contract (called from `syscall_entry`): run the syscall
/// and return the pending fatal status for the asm's `int 0x81` exit path.
#[unsafe(no_mangle)]
extern "C" fn handle_syscall(tf: &mut TrapFrame) -> i32 {
    let frame = unsafe { kor::UserTrapFrame::from_raw(tf as *mut TrapFrame as *mut u8) };
    let Some(frame) = frame else { return 0 };
    crate::syscall::user_trap_enter(frame);
    let nr = tf.rax as usize;
    let args = [
        tf.rdi as usize,
        tf.rsi as usize,
        tf.rdx as usize,
        tf.r10 as usize,
        tf.r8 as usize,
        tf.r9 as usize,
    ];
    tf.rax = crate::syscall::dispatch(nr, &args) as u64;
    let fatal = crate::syscall::user_trap_status(frame);
    crate::syscall::user_trap_leave(frame);
    fatal.unwrap_or(0)
}

/// Application dispatch contract called by the IDT stubs.
#[unsafe(no_mangle)]
extern "C" fn trap_dispatch(tf: &mut TrapFrame) {
    // Process exit vector (also the native-syscall fatal path): `rax`
    // carries the exit status.
    if tf.vector == 0x81 {
        crate::sched::exit_current(tf.rax as i32);
    }

    // LAPIC timer vector (acknowledged by the stub): tick and preempt —
    // kernel mode only; user mode runs without scheduler preemption.
    if tf.vector == 0x20 {
        if (tf.cs & 3) == 0 {
            kor_arch::increment_tick();
            kor_arch::handle_tick();
            if kor_sched::is_ready() {
                kor_sched::timer_tick();
                kor_sched::preempt();
            }
        }
        return;
    }

    let from_user = (tf.cs & 3) == 3;

    // System call via int 0x80 from user mode.
    if tf.vector == 0x80 && from_user {
        let frame = unsafe { kor::UserTrapFrame::from_raw(tf as *mut TrapFrame as *mut u8) };
        if let Some(frame) = frame {
            crate::syscall::user_trap_enter(frame);
            let nr = tf.rax as usize;
            let args = [
                tf.rdi as usize,
                tf.rsi as usize,
                tf.rdx as usize,
                tf.r10 as usize,
                tf.r8 as usize,
                tf.r9 as usize,
            ];
            tf.rax = crate::syscall::dispatch(nr, &args) as u64;
            // PC-advance policy (x86_64-native number): rt_sigreturn
            // restores its own saved RIP.
            const NR_RT_SIGRETURN: usize = 15;
            if nr != NR_RT_SIGRETURN {
                tf.rip += 2;
            }
            crate::syscall::user_trap_leave(frame);
        }
        return;
    }

    // Page fault from user mode: report the fault address (CR2) and stop.
    if tf.vector == 14 && from_user {
        let cr2: u64;
        unsafe {
            core::arch::asm!("mov {}, cr2", out(reg) cr2);
        }
        println!("");
        println!("=== USER PAGE FAULT ===");
        println!("RIP:    {:#018x}", tf.rip);
        println!("CR2:    {:#018x}", cr2);
        println!("Error:  {:#x}", tf.error_code);
        println!("RSP:    {:#018x}", tf.rsp);
        panic!("user-mode page fault at rip={:#x} cr2={:#x}", tf.rip, cr2);
    }

    report_and_panic(tf);
}

/// Print the frame and panic (shared fatal path).
fn report_and_panic(tf: &TrapFrame) -> ! {
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
