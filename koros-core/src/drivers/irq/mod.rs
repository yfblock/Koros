//! External-interrupt handler registry.
//!
//! Device drivers register a handler for their interrupt source number; the
//! per-arch interrupt-controller code (e.g. the riscv PLIC) claims an
//! interrupt and calls [`handle`] to dispatch it.

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use spin::Mutex;

type Handler = Box<dyn Fn() + Send + Sync>;

static HANDLERS: Mutex<Vec<(u32, Handler)>> = Mutex::new(Vec::new());

/// Register `handler` for interrupt source `irq`.
pub fn register(irq: u32, handler: Handler) {
    HANDLERS.lock().push((irq, handler));
}

/// Dispatch interrupt source `irq` to its registered handler, if any.
pub fn handle(irq: u32) {
    let handlers = HANDLERS.lock();
    for (num, handler) in handlers.iter() {
        if *num == irq {
            handler();
        }
    }
}
