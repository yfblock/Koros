//! Composition-owned device registries.
//!
//! The block-device registry, external-IRQ handler registry, and mount table
//! all live here (in the binary crate) rather than in globals buried in
//! libraries.

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::Mutex;

use kor::BlockDevice;
use kor_fs::mount::MountTable;

/// Registry of probed block devices.
pub struct BlockRegistry {
    devices: Mutex<Vec<Arc<dyn BlockDevice>>>,
}

impl BlockRegistry {
    pub const fn new() -> Self {
        Self { devices: Mutex::new(Vec::new()) }
    }
    pub fn register(&self, dev: Arc<dyn BlockDevice>) {
        self.devices.lock().push(dev);
    }
    pub fn first(&self) -> Option<Arc<dyn BlockDevice>> {
        self.devices.lock().first().cloned()
    }
}

/// Singleton block-device registry.
pub static BLOCKS: BlockRegistry = BlockRegistry::new();

type Handler = Box<dyn Fn() + Send + Sync>;

/// Registry of external-IRQ handlers.
pub struct IrqRegistry {
    handlers: Mutex<Vec<(u32, Handler)>>,
}

impl IrqRegistry {
    pub const fn new() -> Self {
        Self { handlers: Mutex::new(Vec::new()) }
    }
    pub fn register(&self, irq: u32, handler: Handler) {
        self.handlers.lock().push((irq, handler));
    }
    pub fn handle(&self, irq: u32) {
        let handlers = self.handlers.lock();
        for (num, handler) in handlers.iter() {
            if *num == irq {
                handler();
            }
        }
    }
}

/// Singleton IRQ-handler registry.
pub static IRQS: IrqRegistry = IrqRegistry::new();

/// Singleton mount table.
pub static MOUNTS: Mutex<MountTable> = Mutex::new(MountTable::new());
