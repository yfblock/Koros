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
    CALLBACKS.get().copied().expect("TrapCallbacks not installed")
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
