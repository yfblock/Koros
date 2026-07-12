//! Trap handling for korix — installs exception vectors that handle:
//! - Syscalls from user mode (ecall/svc)
//! - Page faults (instruction/data abort)
//! - Timer interrupts
//! - External IRQs

/// Initialise trap vectors. Called once per CPU.
pub fn init() {
    // kor-arch's trap_init() already installs the base trap vectors.
    // korix adds user-mode exception handling on top.
    // For now, we reuse kor-arch's trap vectors which call the
    // installed TrapCallbacks (on_timer, on_external).
    // User-mode syscall/page-fault handling will be added by
    // extending the trap vector table.
}

/// Saved register state on trap entry.
#[repr(C)]
pub struct TrapFrame {
    pub regs: [u64; 31],
    pub sp: u64,
    pub pc: u64,
    pub pstate: u64,
    pub esr: u64,
    pub far: u64,
}

/// Main trap dispatch function — called by arch-specific trap entry code.
#[unsafe(no_mangle)]
pub extern "C" fn trap_dispatch(frame: *mut TrapFrame) {
    let f = unsafe { &mut *frame };

    // Dispatch based on exception cause
    // (Architecture-specific EC decoding happens here)
    // For now, just log the trap
    kor::println!(
        "[korix] trap: pc={:#x} esr={:#x} far={:#x}",
        f.pc, f.esr, f.far
    );

    // Advance past the faulting instruction
    f.pc += 4;
}

fn handle_syscall(frame: &mut TrapFrame) {
    let num = frame.regs[0] as usize;
    let args = [
        frame.regs[1] as usize,
        frame.regs[2] as usize,
        frame.regs[3] as usize,
        frame.regs[4] as usize,
        frame.regs[5] as usize,
        frame.regs[6] as usize,
    ];
    let ret = crate::syscall::dispatch(num, &args);
    frame.regs[0] = ret as u64;
    frame.pc += 4;
}

fn handle_page_fault(frame: &mut TrapFrame) {
    kor::println!("[korix] page fault: pc={:#x} far={:#x}", frame.pc, frame.far);
}
