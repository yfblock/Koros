#![no_std]
//! Bottom layer of the flat-composable kernel: generic OS abstractions.
//!
//! Traits (`ArchProvider`, `InterruptController`, `TrapCallbacks`, `Console`,
//! `BlockDevice`, `DeviceDriver`, `INode`/`SuperBlock`), the opaque
//! `TaskContext`, address/region/FDT/cmdline/config helpers, and the trait
//! object registries installed at boot.  No arch-specific code, no
//! subsystem implementations.

extern crate alloc;

pub mod addr;
pub mod arch;
pub mod block;
pub mod boot_stack;
pub mod cmdline;
pub mod config;
pub mod console;
pub mod driver;
pub mod fdt;
pub mod interrupt;
pub mod irq;
pub mod mapping;
pub mod regions;
pub mod smp;
pub mod time;
pub mod trap_callbacks;
pub mod vfs;

// Convenience re-exports at the crate root.
pub use arch::{current, install, phys_to_virt, virt_to_phys, ArchProvider, TaskContext};
pub use block::{BlockDevice, BlockError};
pub use console::{install_console, putc, Console};
pub use interrupt::{controller, enable_device_irq, install_controller, InterruptController};
pub use mapping::{MapError, MapSize, MappingFlags};
pub use trap_callbacks::{
    callbacks, dispatch_external, install_callbacks, on_timer, TrapCallbacks,
};
pub use vfs::{FileType, FsError, FsInfo, INode, Metadata, SuperBlock};

/// Kernel image physical range from linker symbols `_skernel`..`_end`.
/// Used by the composition layer to clip the kernel image out of the frame
/// allocator.
pub fn kernel_phys_range() -> (usize, usize) {
    unsafe extern "C" {
        static _skernel: u8;
        static _end: u8;
    }
    // The linker symbols are virtual (high-half) addresses; subtract the
    // kernel offset to get the physical range used to clip the frame allocator.
    let ko = arch::current().kernel_offset();
    unsafe {
        (
            &_skernel as *const _ as usize - ko,
            &_end as *const _ as usize - ko,
        )
    }
}
