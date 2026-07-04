//! Device-tree driven driver model.
//!
//! A [`DeviceDriver`] declares the `compatible` strings it binds to. At boot
//! the binary crate supplies a registry (a `&[&dyn DeviceDriver]`) and calls
//! [`probe_fdt`], which walks the flattened device tree, matches each node's
//! `compatible` against the registry, and probes the first matching driver
//! with the node's resources.
//!
//! The abstraction and the driver *implementations* live in `koros-core`; the
//! concrete registry (which drivers to enable) is assembled by the `koros`
//! binary crate.

use fdt::properties::Compatible;
use fdt::Fdt;

use crate::println;

/// Resources extracted from a matched device-tree node.
pub struct DtDevice {
    /// Physical base address of the node's first `reg` entry.
    pub reg_base: usize,
    /// Size in bytes of that `reg` entry.
    pub reg_size: usize,
    /// First `interrupts` cell (the interrupt-controller source number), or 0.
    pub irq: u32,
}

/// Error returned by a driver's [`DeviceDriver::probe`].
#[derive(Debug)]
pub enum DriverError {
    /// The device could not be initialised.
    Probe,
    /// The node lacked a resource the driver needs.
    NoResource,
    /// The device is recognised but not supported.
    Unsupported,
}

/// A driver that binds to device-tree nodes by `compatible` string.
pub trait DeviceDriver: Sync {
    /// The `compatible` strings this driver matches.
    fn compatible(&self) -> &'static [&'static str];

    /// Initialise the device described by `dev`.
    fn probe(&self, dev: &DtDevice) -> Result<(), DriverError>;
}

/// Walk the flattened device tree at `fdt_base`, matching each node against
/// `drivers` by `compatible` and probing the first match.
pub fn probe_fdt(fdt_base: usize, drivers: &[&dyn DeviceDriver]) {
    if fdt_base == 0 {
        return;
    }
    // SAFETY: `fdt_base` is the bootloader-provided DTB pointer.
    let fdt = match unsafe { Fdt::from_ptr_unaligned(fdt_base as *const u8) } {
        Ok(fdt) => fdt,
        Err(_) => {
            println!("driver: no valid device tree at {:#x}", fdt_base);
            return;
        }
    };

    for (_depth, node) in fdt.all_nodes() {
        // Only consider nodes with a memory-mapped `reg`.
        let (base, size) = match node
            .reg()
            .and_then(|reg| reg.iter::<u64, u64>().flatten().next())
        {
            Some(entry) => (entry.address as usize, entry.len as usize),
            None => continue,
        };

        // First `interrupts` cell, if present (big-endian u32).
        let irq = node
            .raw_property("interrupts")
            .and_then(|p| p.value.get(0..4))
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            .unwrap_or(0);

        for drv in drivers {
            let matched = node
                .property::<Compatible>()
                .map(|c| c.all().any(|s| drv.compatible().contains(&s)))
                .unwrap_or(false);
            if matched {
                if let Err(e) = drv.probe(&DtDevice {
                    reg_base: base,
                    reg_size: size,
                    irq,
                }) {
                    println!("driver: probe failed at {:#x}: {:?}", base, e);
                }
                break;
            }
        }
    }
}
