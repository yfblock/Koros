#![no_std]
//! Bottom layer of the flat-composable kernel: generic OS abstractions.
//!
//! Traits (`Console`,
//! `BlockDevice`, `DeviceDriver`, `INode`/`SuperBlock`), the opaque
//! `TaskContext`, address/region/FDT/cmdline/config helpers, and the trait
//! object registries installed at boot.  No arch-specific code, no
//! subsystem implementations.

extern crate alloc;

pub mod addr;
pub use addr::{KernelAddr, PhysAddr, VirtAddr, install_translation};
// Device-register bit-field definitions (tock-registers re-export).
// Register access goes through the address newtypes; this gives drivers
// `register_bitfields!` for named fields (e.g. `LSR [THRE 5, DR 0]`)
// and the native `FIELD.is_set(value)` helper.
pub use tock_registers::RegisterLongName;
pub use tock_registers::fields::Field;
pub use tock_registers::register_bitfields;

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
pub mod vfs;

// Convenience re-exports at the crate root.
pub use block::{BlockDevice, BlockError};
pub use console::{Console, install_console, putc};
pub use mapping::{MapError, MapSize, MappingFlags};
pub use vfs::{FileSystemDriver, FileType, FsError, FsInfo, INode, Metadata, SuperBlock};
