#![no_std]
//! Bottom layer of the flat-composable kernel: generic OS abstractions.
//!
//! Traits (`TrapCallbacks`, `Console`,
//! `BlockDevice`, `DeviceDriver`, `INode`/`SuperBlock`), the opaque
//! `TaskContext`, address/region/FDT/cmdline/config helpers, and the trait
//! object registries installed at boot.  No arch-specific code, no
//! subsystem implementations.

extern crate alloc;

pub mod addr;
pub mod mmio;
pub use addr::{KernelAddr, PhysAddr, VirtAddr, install_translation};
pub mod block;
pub mod cmdline;
pub mod config;
pub mod console;
pub mod driver;
pub mod fdt;
pub mod frame;
pub mod mapping;
pub mod regions;
pub mod smp;
pub mod trap_callbacks;
pub mod user_trap;
pub mod vfs;

// Convenience re-exports at the crate root.
pub use block::{BlockDevice, BlockError};
pub use console::{Console, install_console, putc};
pub use mapping::{MapError, MapSize, MappingFlags};
pub use trap_callbacks::{
    TrapCallbacks, callbacks, install_callbacks, install_syscall_handler, on_timer, syscall_handler,
};
pub use user_trap::UserTrapFrame;
pub use vfs::{FileSystemDriver, FileType, FsError, FsInfo, INode, Metadata, SuperBlock};
