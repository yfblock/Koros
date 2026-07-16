//! Composition-owned device registries.

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::Mutex;

use kor::BlockDevice;

pub struct BlockRegistry {
    devices: Mutex<Vec<Arc<dyn BlockDevice>>>,
}

impl BlockRegistry {
    pub const fn new() -> Self {
        Self {
            devices: Mutex::new(Vec::new()),
        }
    }

    pub fn register(&self, device: Arc<dyn BlockDevice>) {
        self.devices.lock().push(device);
    }

    pub fn first(&self) -> Option<Arc<dyn BlockDevice>> {
        self.devices.lock().first().cloned()
    }
}

pub static BLOCKS: BlockRegistry = BlockRegistry::new();

type Handler = Box<dyn Fn() + Send + Sync>;

pub struct IrqRegistry {
    handlers: Mutex<Vec<(u32, Handler)>>,
}

impl IrqRegistry {
    pub const fn new() -> Self {
        Self {
            handlers: Mutex::new(Vec::new()),
        }
    }

    pub fn register(&self, irq: u32, handler: Handler) {
        self.handlers.lock().push((irq, handler));
    }

    pub fn handle(&self, irq: u32) {
        for (registered, handler) in self.handlers.lock().iter() {
            if *registered == irq {
                handler();
            }
        }
    }
}

pub static IRQS: IrqRegistry = IrqRegistry::new();
