//! Architecture implementations (boot, trap, mm, IRQ, timer, SMP, context
//! switch, console, interrupt controllers) for all four targets, plus the
//! register-level UART drivers.  Each target selects its module via
//! `cfg_select`; the binary crate installs the matching
//! `InterruptController` / `Console` at boot.
#![no_std]
#![allow(bad_asm_style)]

use core::sync::atomic::{AtomicU64, Ordering};
extern crate alloc;

cfg_select! {
    target_arch = "riscv64"      => { pub mod riscv64; }
    target_arch = "x86_64"       => { pub mod x86_64; }
    target_arch = "aarch64"      => { pub mod aarch64; }
}

cfg_select! {
    target_arch = "riscv64"      => { pub use self::riscv64::context::TaskContext; }
    target_arch = "x86_64"       => { pub use self::x86_64::context::TaskContext; }
    target_arch = "aarch64"      => { pub use self::aarch64::context::TaskContext; }
}

cfg_select! {
    target_arch = "riscv64"      => { pub use self::riscv64::trap::TrapFrame; }
    target_arch = "x86_64"       => { pub use self::x86_64::trap::TrapFrame; }
    target_arch = "aarch64"      => { pub use self::aarch64::trap::TrapFrame; }
}

cfg_select! {
    target_arch = "riscv64"      => { pub use self::riscv64::pte; }
    target_arch = "x86_64"       => { pub use self::x86_64::pte; }
    target_arch = "aarch64"      => { pub use self::aarch64::pte; }
}

pub mod interrupt;
pub mod pagetable;

// --- Page-mapping types (moved from kor) ------------------------------------

/// Mapping permissions for `map`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappingFlags(u8);

impl MappingFlags {
    /// Readable.
    pub const R: Self = Self(1 << 0);
    /// Writable.
    pub const W: Self = Self(1 << 1);
    /// Executable.
    pub const X: Self = Self(1 << 2);
    /// User-mode accessible.
    pub const U: Self = Self(1 << 3);
    /// Kernel read/write/execute (supervisor-only).
    pub const KERNEL_RWX: Self = Self(Self::R.0 | Self::W.0 | Self::X.0);

    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl core::ops::BitOr for MappingFlags {
    type Output = Self;
    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// Page size for a mapping operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapSize {
    /// 4 KiB base page.
    Page4K,
    /// 2 MiB large page (where the arch supports it).
    Page2M,
}

/// Errors from `map`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    /// Frame or table allocation failed.
    OutOfMemory,
    /// An explicit mapping already occupies the range.
    BlockedByExistingMapping,
    /// The operation is not supported on this architecture.
    Unsupported,
}

pub use interrupt::{InterruptController, enable_device_irq};

/// Dispatch pending external IRQs through the active controller: device
/// IRQs go to `handle_irq`; a controller-delivered timer interrupt (GIC
/// PPI) calls `timer_tick`.
pub fn dispatch_external(handle_irq: &mut dyn FnMut(u32), timer_tick: &mut dyn FnMut()) {
    interrupt_controller().dispatch_external(handle_irq, timer_tick);
}

pub mod boot;
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

/// Restore a captured user context into the active trap frame.
///
/// # Safety
/// `frame` must point to the active architecture trap frame.

/// Redirect an active user context to a Linux signal handler.
///
/// # Safety
/// `frame` must point to the active architecture trap frame and all supplied
/// addresses must already have been validated as user addresses.

/// Resume a captured user context without returning to its original trap stack.
///
/// # Safety
/// The caller must have activated the context's address space and the context
/// must contain validated user PC, SP, and status values.

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

/// The `InterruptController` for the current target (polling stub on x86_64).
pub fn interrupt_controller() -> &'static dyn InterruptController {
    cfg_select! {
        target_arch = "riscv64"      => { &riscv64::ic::PLIC }
        target_arch = "x86_64"       => { &x86_64::ic::IC }
        target_arch = "aarch64"      => { &aarch64::ic::GIC }
    }
}

/// The `Console` for the current target.
pub fn console() -> &'static dyn kor::Console {
    cfg_select! {
        target_arch = "riscv64"      => { &riscv64::console::CONSOLE }
        target_arch = "x86_64"       => { &x86_64::console::CONSOLE }
        target_arch = "aarch64"      => { &aarch64::console::CONSOLE }
    }
}

/// Discover the console base from the device tree (no-op where the console
/// is not FDT-described, e.g. x86_64 legacy port I/O).  Call once, before
/// `install_console(console())`.
pub fn console_init(dtb: usize) {
    cfg_select! {
        target_arch = "riscv64"      => { riscv64::console::console_init(dtb) }
        target_arch = "x86_64"       => { x86_64::console::console_init(dtb) }
        target_arch = "aarch64"      => { aarch64::console::console_init(dtb) }
    }
}

/// The console base in effect (after [`console_init`]); port number on
/// x86_64.
pub fn console_base() -> usize {
    cfg_select! {
        target_arch = "riscv64"      => { riscv64::console::CONSOLE.base.load(core::sync::atomic::Ordering::Relaxed) }
        target_arch = "x86_64"       => { x86_64::console::CONSOLE.base as usize }
        target_arch = "aarch64"      => { aarch64::console::CONSOLE.base.load(core::sync::atomic::Ordering::Relaxed) }
    }
}

// ---------------------------------------------------------------------------
// Static dispatch facade: one cfg-selected call per arch operation.
// Consumers call these directly — no trait objects, no runtime registry.
// ---------------------------------------------------------------------------

cfg_select! {
    target_arch = "riscv64"      => { pub use self::riscv64::mm::{kernel_offset, phys_to_virt, virt_to_phys, dtb_ptr, boot_cmdline}; }
    target_arch = "x86_64"       => { pub use self::x86_64::mm::{kernel_offset, phys_to_virt, virt_to_phys, dtb_ptr, boot_cmdline}; }
    target_arch = "aarch64"      => { pub use self::aarch64::mm::{kernel_offset, phys_to_virt, virt_to_phys, dtb_ptr, boot_cmdline}; }
}

cfg_select! {
    target_arch = "riscv64"      => { pub use self::riscv64::{trap, irq};
pub use self::riscv64::time::{now, now_ticks, timer_hz};
pub use self::riscv64::smp::{cpu_id, wait_for_interrupt, start_secondaries, shutdown}; }
    target_arch = "x86_64"       => { pub use self::x86_64::{trap, irq};
pub use self::x86_64::time::{now, now_ticks, timer_hz};
pub use self::x86_64::smp::{cpu_id, wait_for_interrupt, start_secondaries, shutdown}; }
    target_arch = "aarch64"      => { pub use self::aarch64::{trap, irq};
pub use self::aarch64::time::{now, now_ticks, timer_hz};
pub use self::aarch64::smp::{cpu_id, wait_for_interrupt, start_secondaries, shutdown}; }
}

// --- Timer tick counter (was kor::time) ------------------------------------

/// Timer ticks since boot (incremented by the app trap dispatch).
static TICKS: AtomicU64 = AtomicU64::new(0);

/// Timer interrupts per second (the tick rate the arch timer programs).
pub const TICK_HZ: u64 = 100;

/// Increment the tick counter (called from the app trap dispatch on timer).
#[inline]
pub fn increment_tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

/// Ticks elapsed since boot.
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Detect free memory regions through the current architecture's firmware
/// interface (FDT / Multiboot).
pub fn detect_memory_regions(add_region: &mut dyn FnMut(usize, usize)) {
    cfg_select! {
        target_arch = "riscv64"      => { self::riscv64::mm::init(add_region) }
        target_arch = "x86_64"       => { self::x86_64::mm::init(add_region) }
        target_arch = "aarch64"      => { self::aarch64::mm::init(add_region) }
    }
}

/// Program the timer on the current CPU.
pub fn timer_init() {
    cfg_select! {
        target_arch = "riscv64"      => { self::riscv64::time::init() }
        target_arch = "x86_64"       => { self::x86_64::time::init() }
        target_arch = "aarch64"      => { self::aarch64::time::init() }
    }
}

/// Acknowledge the timer interrupt (per-arch EOI / reprogram).
pub fn handle_tick() {
    cfg_select! {
        target_arch = "riscv64"      => { self::riscv64::time::handle_tick() }
        target_arch = "x86_64"       => { self::x86_64::time::handle_tick() }
        target_arch = "aarch64"      => { self::aarch64::time::handle_tick() }
    }
}

/// Enable interrupts on the current CPU.
pub fn irq_enable() {
    cfg_select! {
        target_arch = "riscv64"      => { self::riscv64::irq::enable() }
        target_arch = "x86_64"       => { self::x86_64::irq::enable() }
        target_arch = "aarch64"      => { self::aarch64::irq::enable() }
    }
}

/// Disable interrupts on the current CPU.
pub fn irq_disable() {
    cfg_select! {
        target_arch = "riscv64"      => { self::riscv64::irq::disable() }
        target_arch = "x86_64"       => { self::x86_64::irq::disable() }
        target_arch = "aarch64"      => { self::aarch64::irq::disable() }
    }
}

/// Whether interrupts are currently enabled on this CPU.
pub fn irq_is_enabled() -> bool {
    cfg_select! {
        target_arch = "riscv64"      => { self::riscv64::irq::is_enabled() }
        target_arch = "x86_64"       => { self::x86_64::irq::is_enabled() }
        target_arch = "aarch64"      => { self::aarch64::irq::is_enabled() }
    }
}

/// Kernel image physical range from linker symbols (clips the frame allocator).
pub fn kernel_phys_range() -> (usize, usize) {
    unsafe extern "C" {
        static _skernel: u8;
        static _end: u8;
    }
    let ko = kernel_offset();
    // SAFETY: linker symbols are static; reading their addresses is sound.
    unsafe {
        (
            &_skernel as *const _ as usize - ko,
            &_end as *const _ as usize - ko,
        )
    }
}

/// Bring up every secondary CPU through the arch bring-up stub.
pub fn boot_secondaries() -> usize {
    let started = start_secondaries();
    if started > 0 {
        kor::smp::register_online();
    }
    started
}

// Provided by the application binary; resolved at link time.
unsafe extern "C" {
    /// Secondary-CPU entry, called from the arch secondary bring-up stub.
    pub fn secondary_entry(cpu_id: usize) -> !;
}

/// Zeroed task context for a new kernel task.
pub fn task_context_zero() -> TaskContext {
    TaskContext::zero()
}

/// Initialize a task context to enter `entry` with `stack_top`.
pub fn task_context_init(ctx: &mut TaskContext, entry: usize, stack_top: usize) {
    ctx.init(entry, stack_top)
}

/// Switch from `prev` to `next`.
///
/// # Safety
/// `prev`/`next` must point to valid, properly-aligned contexts owned by
/// tasks that are not concurrently being switched.
pub unsafe fn context_switch(prev: *mut TaskContext, next: *const TaskContext) {
    // SAFETY: forwarded under the same contract; the re-exported type is
    // exactly the active arch's context layout.
    unsafe {
        cfg_select! {
            target_arch = "riscv64"      => { self::riscv64::context::context_switch(prev, next) }
            target_arch = "x86_64"       => { self::x86_64::context::context_switch(prev, next) }
            target_arch = "aarch64"      => { self::aarch64::context::context_switch(prev, next) }
        }
    }
}

/// Run `f` with interrupts disabled, restoring the previous state afterwards.
pub fn irq_without<R>(f: impl FnOnce() -> R) -> R {
    let was_enabled = irq_is_enabled();
    irq_disable();
    let result = f();
    if was_enabled {
        irq_enable();
    }
    result
}

// --- User-context dispatch (compile-time per-arch) ------------------------

/// Capture a user-mode trap frame through the selected architecture.
///
/// # Safety
/// `frame` must point to the active architecture trap frame.
pub unsafe fn capture_user_context(frame: *mut u8) -> Option<UserTrapContext> {
    cfg_select! {
        target_arch = "riscv64" => { unsafe { self::riscv64::trap::capture_user_context(frame) } }
        target_arch = "aarch64" => { unsafe { self::aarch64::trap::capture_user_context(frame) } }
        target_arch = "x86_64" => { unsafe { self::x86_64::trap::capture_user_context(frame) } }
        _ => { let _ = frame; None }
    }
}

/// Restore a captured user context into the active trap frame.
///
/// # Safety
/// `frame` must point to the active architecture trap frame.
pub unsafe fn restore_user_context(frame: *mut u8, context: &UserTrapContext) -> bool {
    cfg_select! {
        target_arch = "riscv64" => { unsafe { self::riscv64::trap::restore_user_context(frame, context) } }
        target_arch = "aarch64" => { unsafe { self::aarch64::trap::restore_user_context(frame, context) } }
        target_arch = "x86_64" => { unsafe { self::x86_64::trap::restore_user_context(frame, context) } }
        _ => { let _ = (frame, context); false }
    }
}

/// Redirect an active user context to a signal handler.
///
/// # Safety
/// `frame` must point to the active architecture trap frame.
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
        target_arch = "riscv64" => { unsafe { self::riscv64::trap::setup_user_signal(frame, handler, restorer, signal_frame, signal, siginfo, ucontext) } }
        target_arch = "aarch64" => { unsafe { self::aarch64::trap::setup_user_signal(frame, handler, restorer, signal_frame, signal, siginfo, ucontext) } }
        target_arch = "x86_64" => { unsafe { self::x86_64::trap::setup_user_signal(frame, handler, restorer, signal_frame, signal, siginfo, ucontext) } }
        _ => { let _ = (frame, handler, restorer, signal_frame, signal, siginfo, ucontext); false }
    }
}

/// Resume a captured user context without returning to the original trap stack.
///
/// # Safety
/// The caller must have activated the context's address space.
pub unsafe fn resume_user_context(context: &UserTrapContext) -> ! {
    cfg_select! {
        target_arch = "riscv64" => { unsafe { self::riscv64::trap::resume_user_context(context) } }
        target_arch = "aarch64" => { unsafe { self::aarch64::trap::resume_user_context(context) } }
        target_arch = "x86_64" => { unsafe { self::x86_64::trap::resume_user_context(context) } }
        _ => { let _ = context; loop { core::hint::spin_loop(); } }
    }
}
