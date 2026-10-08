//! Architecture implementations (boot, trap, mm, IRQ, timer, SMP, context
//! switch, console, interrupt controllers) for all four targets, plus the
//! register-level UART drivers.  Each target selects its module via
//! `cfg_select`; the binary crate installs the matching `ArchProvider` /
//! `InterruptController` / `Console` at boot.
#![no_std]
#![allow(bad_asm_style)]

extern crate alloc;

cfg_select! {
    target_arch = "riscv64"      => { pub mod riscv64; }
    target_arch = "x86_64"       => { pub mod x86_64; }
    target_arch = "aarch64"      => { pub mod aarch64; }
    target_arch = "loongarch64" => { pub mod loongarch64; }
}

pub mod uart;
/// Architecture-neutral snapshot of a user trap frame.
///
/// `regs` uses the architecture's native integer-register numbering: x0-x31
/// on RISC-V, x0-x30 on AArch64, and the order documented by the x86 trap
/// module. `status` is sanitized by each architecture before restoration.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct UserTrapContext {
    pub regs: [usize; 32],
    pub pc: usize,
    pub status: usize,
    pub sp: usize,
    pub tls: usize,
    pub fp_callee: [u64; 12],
    pub fcsr: usize,
}
impl UserTrapContext {
    /// Construct a zeroed context for the first entry into a user image.
    pub fn initial(pc: usize, sp: usize) -> Self {
        Self {
            regs: [0; 32],
            pc,
            status: 0,
            sp,
            tls: 0,
            fp_callee: [0; 12],
            fcsr: 0,
        }
    }
}

/// Capture a user-mode trap frame through the selected architecture's typed layout.
///
/// # Safety
/// `frame` must point to the active architecture trap frame.
pub unsafe fn capture_user_context(frame: *mut u8) -> Option<UserTrapContext> {
    cfg_select! {
        target_arch = "riscv64" => { unsafe { riscv64::trap::capture_user_context(frame) } }
        target_arch = "aarch64" => { unsafe { aarch64::trap::capture_user_context(frame) } }
        target_arch = "x86_64" => { unsafe { x86_64::trap::capture_user_context(frame) } }
        _ => { let _ = frame; None }
    }
}

/// Restore a captured user context into the active trap frame.
///
/// # Safety
/// `frame` must point to the active architecture trap frame.
pub unsafe fn restore_user_context(frame: *mut u8, context: &UserTrapContext) -> bool {
    cfg_select! {
        target_arch = "riscv64" => { unsafe { riscv64::trap::restore_user_context(frame, context) } }
        target_arch = "aarch64" => { unsafe { aarch64::trap::restore_user_context(frame, context) } }
        target_arch = "x86_64" => { unsafe { x86_64::trap::restore_user_context(frame, context) } }
        _ => { let _ = (frame, context); false }
    }
}

/// Redirect an active user context to a Linux signal handler.
///
/// # Safety
/// `frame` must point to the active architecture trap frame and all supplied
/// addresses must already have been validated as user addresses.
pub unsafe fn setup_user_signal(
    frame: *mut u8,
    handler: usize,
    restorer: usize,
    signal_frame: usize,
    signal: usize,
    siginfo: usize,
    ucontext: usize,
) -> bool {
    cfg_select! {
        target_arch = "riscv64" => { unsafe { riscv64::trap::setup_user_signal(frame, handler, restorer, signal_frame, signal, siginfo, ucontext) } }
        target_arch = "aarch64" => { unsafe { aarch64::trap::setup_user_signal(frame, handler, restorer, signal_frame, signal, siginfo, ucontext) } }
        target_arch = "x86_64" => { unsafe { x86_64::trap::setup_user_signal(frame, handler, restorer, signal_frame, signal, siginfo, ucontext) } }
        _ => { let _ = (frame, handler, restorer, signal_frame, signal, siginfo, ucontext); false }
    }
}

/// Resume a captured user context without returning to its original trap stack.
///
/// # Safety
/// The caller must have activated the context's address space and the context
/// must contain validated user PC, SP, and status values.
pub unsafe fn resume_user_context(context: &UserTrapContext) -> ! {
    cfg_select! {
        target_arch = "riscv64" => { unsafe { riscv64::trap::resume_user_context(context) } }
        target_arch = "aarch64" => { unsafe { aarch64::trap::resume_user_context(context) } }
        target_arch = "x86_64" => { unsafe { x86_64::trap::resume_user_context(context) } }
        _ => { let _ = context; loop { core::hint::spin_loop(); } }
    }
}

/// Enter a freshly loaded user image through the selected architecture.
///
/// The caller supplies only validated user PC and SP values. Architecture
/// code owns privilege-state sanitization and the non-returning transition.
pub fn enter_user(pc: usize, sp: usize) -> ! {
    let context = UserTrapContext::initial(pc, sp);
    // SAFETY: the composition layer activates the new address space before
    // calling this function; `initial` contains no privileged state.
    unsafe { resume_user_context(&context) }
}

// ---------------------------------------------------------------------------
// Composition helpers: cfg-selected singletons for the binary crate to install.
// ---------------------------------------------------------------------------

/// The `ArchProvider` for the current target.
pub fn provider() -> &'static dyn kor::ArchProvider {
    cfg_select! {
        target_arch = "riscv64"      => { &riscv64::provider::PROVIDER }
        target_arch = "x86_64"       => { &x86_64::provider::PROVIDER }
        target_arch = "aarch64"      => { &aarch64::provider::PROVIDER }
        target_arch = "loongarch64" => { &loongarch64::provider::PROVIDER }
    }
}

/// The `InterruptController` for the current target (stub on x86_64/loongarch64).
pub fn interrupt_controller() -> &'static dyn kor::InterruptController {
    cfg_select! {
        target_arch = "riscv64"      => { &riscv64::ic::PLIC }
        target_arch = "x86_64"       => { &x86_64::ic::IC }
        target_arch = "aarch64"      => { &aarch64::ic::GIC }
        target_arch = "loongarch64" => { &loongarch64::ic::IC }
    }
}

/// The `Console` for the current target.
pub fn console() -> &'static dyn kor::Console {
    cfg_select! {
        target_arch = "riscv64"      => { &riscv64::console::CONSOLE }
        target_arch = "x86_64"       => { &x86_64::console::CONSOLE }
        target_arch = "aarch64"      => { &aarch64::console::CONSOLE }
        target_arch = "loongarch64" => { &loongarch64::console::CONSOLE }
    }
}

// Provided by the application binary; resolved at link time.
unsafe extern "C" {
    /// Secondary-CPU entry, called from the arch secondary bring-up stub.
    pub fn secondary_entry(cpu_id: usize) -> !;
}
