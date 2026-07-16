//! `TrapCallbacks` trait + installed-callback registry + dispatch entry points.

use spin::Once;

use crate::interrupt::controller;

/// Behaviour the arch trap handlers invoke on timer and external interrupts.
pub trait TrapCallbacks: Send + Sync {
    fn on_timer(&self);
    fn on_external(&self, irq: u32);
}

static CALLBACKS: Once<&'static dyn TrapCallbacks> = Once::new();

/// Install the trap callback set.  Call once before enabling interrupts.
pub fn install_callbacks(cb: &'static dyn TrapCallbacks) {
    CALLBACKS.call_once(|| cb);
}

/// The installed trap callbacks.  Panics if [`install_callbacks`] was not called.
pub fn callbacks() -> &'static dyn TrapCallbacks {
    CALLBACKS
        .get()
        .copied()
        .expect("TrapCallbacks not installed")
}

/// Invoke the timer callback (called from arch timer trap handlers).
pub fn on_timer() {
    callbacks().on_timer();
}

/// Dispatch pending external IRQs through the installed controller.
pub fn dispatch_external() {
    let cb = callbacks();
    if let Some(ic) = controller() {
        ic.dispatch_external(cb);
    }
}

// ---------------------------------------------------------------------------
// Syscall handler registry (optional — installed by composition layers that
// support user-mode processes, e.g. korix; koros leaves it unset).
// ---------------------------------------------------------------------------

/// Signature of a syscall handler: `(nr, args[0..6]) -> ret`.
pub type SyscallHandler = fn(usize, &[usize]) -> isize;

static SYSCALL_HANDLER: Once<SyscallHandler> = Once::new();

/// Install the syscall handler.  Call once before entering user mode.
pub fn install_syscall_handler(handler: SyscallHandler) {
    SYSCALL_HANDLER.call_once(|| handler);
}

/// The installed syscall handler, or `None` if user mode is not supported.
pub fn syscall_handler() -> Option<SyscallHandler> {
    SYSCALL_HANDLER.get().copied()
}

/// Architecture-neutral hooks around a user syscall trap. The process
/// composition layer installs these to expose the active frame and deliver
/// pending signals; kernel-only compositions leave them unset.
pub type UserTrapHook = unsafe fn(*mut u8);
static USER_TRAP_ENTER: Once<UserTrapHook> = Once::new();
static USER_TRAP_LEAVE: Once<UserTrapHook> = Once::new();
pub type UserTrapStatusHook = unsafe fn(*mut u8) -> Option<i32>;
static USER_TRAP_STATUS: Once<UserTrapStatusHook> = Once::new();
pub type UserExitHook = fn(i32) -> !;
static USER_EXIT: Once<UserExitHook> = Once::new();
pub type TaskSwitchHook = fn(usize);
static TASK_SWITCH: Once<TaskSwitchHook> = Once::new();

pub fn install_user_trap_hooks(enter: UserTrapHook, leave: UserTrapHook) {
    USER_TRAP_ENTER.call_once(|| enter);
    USER_TRAP_LEAVE.call_once(|| leave);
}

pub fn install_user_trap_status_hook(hook: UserTrapStatusHook) {
    USER_TRAP_STATUS.call_once(|| hook);
}

pub fn install_user_exit_hook(hook: UserExitHook) {
    USER_EXIT.call_once(|| hook);
}

pub fn user_exit(code: i32) -> ! {
    if let Some(hook) = USER_EXIT.get() {
        hook(code)
    }
    loop {
        crate::arch::current().wait_for_interrupt();
    }
}

pub fn install_task_switch_hook(hook: TaskSwitchHook) {
    TASK_SWITCH.call_once(|| hook);
}

pub fn task_switch(user_stack_top: usize) {
    if let Some(hook) = TASK_SWITCH.get() {
        hook(user_stack_top);
    }
}

/// # Safety
/// `frame` must be the live architecture trap frame for the current task.
pub unsafe fn user_trap_enter(frame: *mut u8) {
    if let Some(hook) = USER_TRAP_ENTER.get() {
        unsafe { hook(frame) };
    }
}

/// # Safety
/// `frame` must be the live architecture trap frame for the current task.
pub unsafe fn user_trap_leave(frame: *mut u8) {
    if let Some(hook) = USER_TRAP_LEAVE.get() {
        unsafe { hook(frame) };
    }
}

/// Return a fatal process status requested by the composition layer.
///
/// # Safety
/// `frame` must be the live architecture trap frame for the current task.
pub unsafe fn user_trap_status(frame: *mut u8) -> Option<i32> {
    USER_TRAP_STATUS
        .get()
        .and_then(|hook| unsafe { hook(frame) })
}
