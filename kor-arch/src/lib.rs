#![no_std]
#![allow(bad_asm_style)]
//! Architecture implementations (boot, trap, mm, page tables, IRQ, timer, SMP,
//! context switch, console, interrupt controllers) for all four targets, plus
//! the register-level UART drivers.  Each target selects its module via
//! `cfg_if`; the binary crate installs the matching `ArchProvider` /
//! `InterruptController` / `Console` at boot.
extern crate alloc;

cfg_if::cfg_if! {
    if #[cfg(target_arch = "riscv64")]      { pub mod riscv64; }
    else if #[cfg(target_arch = "x86_64")]  { pub mod x86_64; }
    else if #[cfg(target_arch = "aarch64")] { pub mod aarch64; }
    else if #[cfg(target_arch = "loongarch64")] { pub mod loongarch64; }
    else { compile_error!("unsupported target_arch"); }
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

/// Capture a user-mode trap frame through the selected architecture's typed layout.
///
/// # Safety
/// `frame` must point to the active architecture trap frame.
pub unsafe fn capture_user_context(frame: *mut u8) -> Option<UserTrapContext> {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "riscv64")] { unsafe { riscv64::trap::capture_user_context(frame) } }
        else if #[cfg(target_arch = "aarch64")] { unsafe { aarch64::trap::capture_user_context(frame) } }
        else if #[cfg(target_arch = "x86_64")] { unsafe { x86_64::trap::capture_user_context(frame) } }
        else { let _ = frame; None }
    }
}

/// Restore a captured user context into the active trap frame.
///
/// # Safety
/// `frame` must point to the active architecture trap frame.
pub unsafe fn restore_user_context(frame: *mut u8, context: &UserTrapContext) -> bool {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "riscv64")] { unsafe { riscv64::trap::restore_user_context(frame, context) } }
        else if #[cfg(target_arch = "aarch64")] { unsafe { aarch64::trap::restore_user_context(frame, context) } }
        else if #[cfg(target_arch = "x86_64")] { unsafe { x86_64::trap::restore_user_context(frame, context) } }
        else { let _ = (frame, context); false }
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
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "riscv64")] { unsafe { riscv64::trap::setup_user_signal(frame, handler, restorer, signal_frame, signal, siginfo, ucontext) } }
        else if #[cfg(target_arch = "aarch64")] { unsafe { aarch64::trap::setup_user_signal(frame, handler, restorer, signal_frame, signal, siginfo, ucontext) } }
        else if #[cfg(target_arch = "x86_64")] { unsafe { x86_64::trap::setup_user_signal(frame, handler, restorer, signal_frame, signal, siginfo, ucontext) } }
        else { let _ = (frame, handler, restorer, signal_frame, signal, siginfo, ucontext); false }
    }
}

/// Resume a captured user context without returning to its original trap stack.
///
/// # Safety
/// The caller must have activated the context's address space and the context
/// must contain validated user PC, SP, and status values.
pub unsafe fn resume_user_context(context: &UserTrapContext) -> ! {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "riscv64")] { unsafe { riscv64::trap::resume_user_context(context) } }
        else if #[cfg(target_arch = "aarch64")] { unsafe { aarch64::trap::resume_user_context(context) } }
        else if #[cfg(target_arch = "x86_64")] { unsafe { x86_64::trap::resume_user_context(context) } }
        else { let _ = context; loop { core::hint::spin_loop(); } }
    }
}

// ---------------------------------------------------------------------------
// Composition helpers: cfg-selected singletons for the binary crate to install.
// ---------------------------------------------------------------------------

/// The `ArchProvider` for the current target.
pub fn provider() -> &'static dyn kor::ArchProvider {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "riscv64")]      { &riscv64::provider::PROVIDER }
        else if #[cfg(target_arch = "x86_64")]  { &x86_64::provider::PROVIDER }
        else if #[cfg(target_arch = "aarch64")] { &aarch64::provider::PROVIDER }
        else if #[cfg(target_arch = "loongarch64")] { &loongarch64::provider::PROVIDER }
        else { compile_error!("unsupported target_arch") }
    }
}

/// The `InterruptController` for the current target (stub on x86_64/loongarch64).
pub fn interrupt_controller() -> &'static dyn kor::InterruptController {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "riscv64")]      { &riscv64::ic::PLIC }
        else if #[cfg(target_arch = "x86_64")]  { &x86_64::ic::IC }
        else if #[cfg(target_arch = "aarch64")] { &aarch64::ic::GIC }
        else if #[cfg(target_arch = "loongarch64")] { &loongarch64::ic::IC }
        else { compile_error!("unsupported target_arch") }
    }
}

/// The `Console` for the current target.
pub fn console() -> &'static dyn kor::Console {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "riscv64")]      { &riscv64::console::CONSOLE }
        else if #[cfg(target_arch = "x86_64")]  { &x86_64::console::CONSOLE }
        else if #[cfg(target_arch = "aarch64")] { &aarch64::console::CONSOLE }
        else if #[cfg(target_arch = "loongarch64")] { &loongarch64::console::CONSOLE }
        else { compile_error!("unsupported target_arch") }
    }
}

/// Arch-specific test virtual address for a 4 KiB mapping (page-table self-test).
#[cfg(target_arch = "riscv64")]
pub const TEST_VA_4K: usize = riscv64::page_table::TEST_VA_4K;
#[cfg(target_arch = "x86_64")]
pub const TEST_VA_4K: usize = x86_64::page_table::TEST_VA_4K;
#[cfg(target_arch = "aarch64")]
pub const TEST_VA_4K: usize = aarch64::page_table::TEST_VA_4K;
#[cfg(target_arch = "loongarch64")]
pub const TEST_VA_4K: usize = loongarch64::page_table::TEST_VA_4K;

/// Arch-specific test virtual address for a 2 MiB mapping (page-table self-test).
#[cfg(target_arch = "riscv64")]
pub const TEST_VA_2M: usize = riscv64::page_table::TEST_VA_2M;
#[cfg(target_arch = "x86_64")]
pub const TEST_VA_2M: usize = x86_64::page_table::TEST_VA_2M;
#[cfg(target_arch = "aarch64")]
pub const TEST_VA_2M: usize = aarch64::page_table::TEST_VA_2M;
#[cfg(target_arch = "loongarch64")]
pub const TEST_VA_2M: usize = loongarch64::page_table::TEST_VA_2M;

// Provided by the `koros` binary crate; resolved at link time.
unsafe extern "C" {
    /// Kernel entry, called by the arch boot code after early setup.
    pub fn kernel_main() -> !;
    /// Secondary-CPU entry, called by the arch secondary boot stub.
    pub fn secondary_entry(id: usize) -> !;
}
