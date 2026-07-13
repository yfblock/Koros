//! riscv64 trap handling.
use kor::println;
use core::arch::{asm, global_asm};

#[repr(C)]
struct TrapFrame {
    regs: [usize; 32],
    sstatus: usize,
    sepc: usize,
}

global_asm!(include_str!("trap.S"), tf_size = const core::mem::size_of::<TrapFrame>());

unsafe extern "Rust" { fn kernelvec(); }

#[unsafe(no_mangle)]
extern "C" fn handle_trap(tf: &mut TrapFrame) {
    let scause: usize;
    let stval: usize;
    unsafe {
        asm!("csrr {}, scause", out(reg) scause);
        asm!("csrr {}, stval", out(reg) stval);
    }
    let is_interrupt = (scause >> 63) != 0;
    let code = scause & ((1 << 63) - 1);

    if is_interrupt && code == 5 { kor::on_timer(); return; }
    if is_interrupt && code == 9 { kor::dispatch_external(); return; }

    if !is_interrupt && code == 8 {
        if let Some(handler) = kor::trap_callbacks::syscall_handler() {
            // Set current TrapFrame pointer for fork/clone support
            unsafe extern "Rust" {
                fn korix_set_current_tf(tf: *mut u8);
            }
            unsafe { korix_set_current_tf(tf as *mut TrapFrame as *mut u8); }
            let nr = tf.regs[17];
            let args = [tf.regs[10], tf.regs[11], tf.regs[12],
                        tf.regs[13], tf.regs[14], tf.regs[15]];
            let ret = handler(nr, &args);
            tf.regs[10] = ret as usize;
            // After execve (221), sepc is set to entry point — don't skip ecall.
            if nr != 221 { tf.sepc += 4; }
            return;
        }
    }

    println!("");
    println!("=== TRAP ===");
    println!("scause  = {:#x} ({} #{})", scause, if is_interrupt { "Interrupt" } else { "Exception" }, code);
    println!("stval   = {:#x}", stval);
    println!("sepc    = {:#x}", tf.sepc);
    println!("sstatus = {:#x}", tf.sstatus);
    panic!("Unhandled trap: scause={:#x}, sepc={:#x}", scause, tf.sepc);
}

pub fn init() {
    unsafe { asm!("csrw stvec, {}", in(reg) kernelvec as *const () as usize); }
    println!("trap: stvec set to kernelvec ({:#x})", kernelvec as *const () as usize);
}
