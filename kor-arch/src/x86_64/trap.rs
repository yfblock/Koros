//! x86_64 trap handling.
//!
//! Builds an IDT with 256 entries (each pointing to an assembly stub that
//! saves registers), loads the IDT, and prints trap info on exceptions.

use kor::println;
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
    // Pushed by the CPU — always present in 64-bit long mode, even for
    // same-privilege traps (unlike 32-bit mode, which omits SS:RSP when
    // there is no privilege-level change).
    rsp:    u64,
    ss:     u64,
}

// ---------------------------------------------------------------------------
// GDT + TSS — ring-3 support
// ---------------------------------------------------------------------------

/// 10-byte table descriptor (GDTR/IDTR): 16-bit limit + 64-bit base.
#[repr(C, packed)]
struct Dtr {
    limit: u16,
    base:  u64,
}

/// 8-slot GDT: null, kcode64, kdata, ucode64, udata, tss (2 slots), pad.
#[repr(C, align(16))]
struct Gdt {
    entries: [u64; 8],
}

/// 104-byte 64-bit Task State Segment; only `rsp0` is used here.
#[repr(C, align(16))]
struct Tss {
    _reserved0: u32,
    rsp0_low:   u32,
    rsp0_high:  u32,
    _padding:   [u8; 92],
}

/// Dedicated kernel stack for traps arriving from user mode: the CPU loads
/// RSP from TSS.RSP0 on a ring-3 → ring-0 transition.  16 KiB is ample for
/// non-reentrant (interrupt-gate) handling.
#[repr(align(16))]
struct IntStack([u8; 0x4000]);

static mut GDT: Gdt = Gdt {
    entries: [
        0,                       // null
        0x00af9b000000ffff,      // [1] kcode64  (sel 0x08)
        0x00cf93000000ffff,      // [2] kdata    (sel 0x10)
        0x00affb000000ffff,      // [3] ucode64  (sel 0x1B)
        0x00cff3000000ffff,      // [4] udata    (sel 0x23)
        0,                       // [5] tss low  (sel 0x28) — filled in init()
        0,                       // [6] tss high
        0,                       // pad
    ],
};

static mut TSS: Tss = Tss { _reserved0: 0, rsp0_low: 0, rsp0_high: 0, _padding: [0; 92] };
static mut INT_STACK: IntStack = IntStack([0; 0x4000]);

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
    // Timer interrupts are acknowledged by hardware; korix user-mode runs
    // without the scheduler, so avoid preemption/callback work while in ring 3.
    if tf.vector == 0x20 {
        if (tf.cs & 3) == 0 {
            kor::on_timer();
        }
        return;
    }

    let from_user = (tf.cs & 3) == 3;

    // Native SYSCALL path. The assembly uses synthetic vector 0x100 and
    // builds the same ring-3 return frame consumed by iretq.
    if tf.vector == 0x100 && from_user {
        if let Some(handler) = kor::trap_callbacks::syscall_handler() {
            unsafe extern "Rust" {
                fn korix_set_current_tf(tf: *mut u8);
            }
            unsafe { korix_set_current_tf(tf as *mut TrapFrame as *mut u8); }
            let nr = tf.rax as usize;
            let args = [
                tf.rdi as usize,
                tf.rsi as usize,
                tf.rdx as usize,
                tf.r10 as usize,
                tf.r8 as usize,
                tf.r9 as usize,
            ];
            tf.rax = handler(nr, &args) as u64;
            return;
        }
    }

    // System call via int 0x80 from user mode.
    if tf.vector == 0x80 && from_user {
        if let Some(handler) = kor::trap_callbacks::syscall_handler() {
            unsafe extern "Rust" {
                fn korix_set_current_tf(tf: *mut u8);
            }
            unsafe { korix_set_current_tf(tf as *mut TrapFrame as *mut u8); }
            let nr = tf.rax as usize;
            let args = [
                tf.rdi as usize,
                tf.rsi as usize,
                tf.rdx as usize,
                tf.r10 as usize,
                tf.r8 as usize,
                tf.r9 as usize,
            ];
            let ret = handler(nr, &args);
            tf.rax = ret as u64;
            tf.rip += 2;
            return;
        }
    }

    // Page fault from user mode: report the fault address (CR2) and stop.
    if tf.vector == 14 && from_user {
        let cr2: u64;
        unsafe { core::arch::asm!("mov {}, cr2", out(reg) cr2); }
        println!("");
        println!("=== USER PAGE FAULT ===");
        println!("RIP:    {:#018x}", tf.rip);
        println!("CR2:    {:#018x}", cr2);
        println!("Error:  {:#x}", tf.error_code);
        println!("RSP:    {:#018x}", tf.rsp);
        panic!("user-mode page fault at rip={:#x} cr2={:#x}", tf.rip, cr2);
    }

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
        tf.cs, tf.ss, tf.rsp);

    panic!("Unhandled trap");
}

// ---------------------------------------------------------------------------
// Public init — build IDT, load it
// ---------------------------------------------------------------------------

/// Install the trap handler: build + load the GDT (with user segments) and
/// TSS, reload segment registers, then build + load the IDT.
pub fn init() {
    unsafe {
        let gdt = &raw mut GDT;
        let tss = &raw mut TSS;

        // Build the 16-byte TSS descriptor in GDT slots 5 and 6.
        let tss_base = tss as u64;
        let tss_limit = (core::mem::size_of::<Tss>() - 1) as u64;
        (*gdt).entries[5] = (tss_limit & 0xffff)
            | ((tss_base & 0x00ff_ffff) << 16)
            | (0x89u64 << 40) // P=1, DPL=0, S=0, type=0x9 (avail 64-bit TSS)
            | (((tss_limit >> 16) & 0xf) << 48)
            | (((tss_base >> 24) & 0xff) << 56);
        (*gdt).entries[6] = (tss_base >> 32) & 0xffff_ffff;

        let rsp0 = (&raw const INT_STACK as u64) + core::mem::size_of::<IntStack>() as u64;
        (*tss).rsp0_low = rsp0 as u32;
        (*tss).rsp0_high = (rsp0 >> 32) as u32;

        // Load the GDT, reload segment registers (kcode=0x08, kdata=0x10),
        // and load the task register (TSS selector 0x28).
        let gdtr = Dtr {
            limit: (core::mem::size_of::<Gdt>() - 1) as u16,
            base:  gdt as u64,
        };
        _load_gdt(&raw const gdtr);

        // Build and load the IDT (handler CS = 0x08 = kcode64).
        _init_idt();

        // Enable native SYSCALL. STAR selects kernel CS 0x08; LSTAR points at
        // the assembly entry. Return uses iretq, so only kernel STAR matters.
        let efer_low: u32;
        let efer_high: u32;
        core::arch::asm!(
            "rdmsr",
            in("ecx") 0xC000_0080u32,
            lateout("eax") efer_low,
            lateout("edx") efer_high,
            options(nostack),
        );
        let efer = ((efer_high as u64) << 32) | efer_low as u64 | 1;
        core::arch::asm!(
            "wrmsr",
            in("ecx") 0xC000_0080u32,
            in("eax") efer as u32,
            in("edx") (efer >> 32) as u32,
            options(nostack),
        );
        let star = 0x08u64 << 32;
        core::arch::asm!(
            "wrmsr",
            in("ecx") 0xC000_0081u32,
            in("eax") star as u32,
            in("edx") (star >> 32) as u32,
            options(nostack),
        );
        let lstar = syscall_entry as *const () as u64;
        core::arch::asm!(
            "wrmsr",
            in("ecx") 0xC000_0082u32,
            in("eax") lstar as u32,
            in("edx") (lstar >> 32) as u32,
            options(nostack),
        );
        let sfmask = 0u64;
        core::arch::asm!(
            "wrmsr",
            in("ecx") 0xC000_0084u32,
            in("eax") sfmask as u32,
            in("edx") (sfmask >> 32) as u32,
            options(nostack),
        );
    }
    println!("trap: GDT (user segments) + TSS + IDT loaded");
}

unsafe extern "C" {
    fn _init_idt();
    fn _load_gdt(gdtr: *const Dtr);
    fn syscall_entry();
}
