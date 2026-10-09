//! FDT (Flattened Device Tree) parser — powered by the `fdt` crate.
//!
//! Scans the tree for `/memory` nodes and extracts memory regions.

use alloc::string::String;
use fdt::Fdt;

/// Parse FDT at `fdt_base` (physical address) and call `add_region` for each
/// physical-memory region found under `/memory`.
///
/// Returns the number of regions added.
///
/// # Safety
///
/// `fdt_base` must point to a valid, accessible FDT blob in memory.
pub unsafe fn parse_memory_regions(
    fdt_base: usize,
    mut add_region: impl FnMut(usize, usize),
) -> usize {
    // SAFETY: caller guarantees `fdt_base` points to a valid FDT blob.
    let fdt = match unsafe { Fdt::from_ptr_unaligned(fdt_base as *const u8) } {
        Ok(fdt) => fdt,
        Err(_) => return 0,
    };

    let mem = fdt.root().memory();
    let reg = mem.reg();
    let mut count = 0;
    for entry in reg.iter::<u64, u64>().flatten() {
        let start = entry.address as usize;
        let size = entry.len as usize;
        if size > 0 {
            add_region(start, start + size);
            count += 1;
        }
    }
    count
}

/// Count the CPU nodes under `/cpus` in the FDT (at least 1).
///
/// # Safety
///
/// `fdt_base` must point to a valid, accessible FDT blob in memory.
pub unsafe fn cpu_count(fdt_base: usize) -> usize {
    // SAFETY: caller guarantees `fdt_base` points to a valid FDT blob.
    let fdt = match unsafe { Fdt::from_ptr_unaligned(fdt_base as *const u8) } {
        Ok(fdt) => fdt,
        Err(_) => return 1,
    };
    fdt.root().cpus().iter().count().max(1)
}

/// Read the kernel command line from the FDT `/chosen` node's `bootargs`
/// property, copied into an owned [`String`].
///
/// # Safety
///
/// `fdt_base` must point to a valid, accessible FDT blob in memory.
pub unsafe fn bootargs(fdt_base: usize) -> Option<String> {
    // SAFETY: caller guarantees `fdt_base` points to a valid FDT blob.
    let fdt = unsafe { Fdt::from_ptr_unaligned(fdt_base as *const u8) }.ok()?;
    let args = fdt.root().chosen().bootargs()?;
    Some(String::from(args))
}

/// Look up the `index`-th register address of the first node whose
/// `compatible` property matches any of `compat` (allocation-free).
///
/// Returns the physical base address, or `None` when no matching node (or
/// no such register) exists.
///
/// # Safety
///
/// `fdt_base` must point to a valid, accessible FDT blob in memory.
pub unsafe fn find_compatible_reg(fdt_base: usize, compat: &[&str], index: usize) -> Option<usize> {
    // SAFETY: caller guarantees `fdt_base` points to a valid FDT blob.
    let fdt = unsafe { Fdt::from_ptr_unaligned(fdt_base as *const u8) }.ok()?;
    for node in fdt.all_compatible(compat) {
        let reg = node.reg()?;
        for (position, entry) in reg.iter::<u64, u64>().flatten().enumerate() {
            if position == index {
                return Some(entry.address as usize);
            }
        }
        return None;
    }
    None
}

/// Read the first big-endian `u32` cell of a property value.
fn be_u32(value: &[u8]) -> Option<u32> {
    (value.len() >= 4).then(|| u32::from_be_bytes([value[0], value[1], value[2], value[3]]))
}

/// A discovered ns16550a-family UART (base plus register geometry).
#[derive(Clone, Copy, Debug)]
pub struct UartInfo {
    /// Physical base address.
    pub base: usize,
    /// Register spacing shift (`reg-shift`; 0 = byte-spaced).
    pub reg_shift: u32,
    /// Register access width in bytes (`reg-io-width`; 1/4).
    pub reg_io_width: u32,
}

impl Default for UartInfo {
    fn default() -> Self {
        Self {
            base: 0,
            reg_shift: 0,
            reg_io_width: 1,
        }
    }
}

/// Find the first ns16550a-family UART in the tree and read its geometry
/// (`reg`, `reg-shift`, `reg-io-width`; the latter two default to 0/1).
///
/// # Safety
///
/// `fdt_base` must point to a valid, accessible FDT blob in memory.
/// Find the first ns16550a-family UART in the tree and read its geometry
/// (`reg`, `reg-shift`, `reg-io-width`; the latter two default to 0/1).
///
/// # Safety
///
/// `fdt_base` must point to a valid, accessible FDT blob in memory.
pub unsafe fn find_uart(fdt_base: usize) -> Option<UartInfo> {
    // SAFETY: caller guarantees `fdt_base` points to a valid FDT blob.
    let fdt = unsafe { Fdt::from_ptr_unaligned(fdt_base as *const u8) }.ok()?;
    const COMPAT: &[&str] = &["snps,dw-apb-uart", "ns16550a", "ns16550", "uart8250"];
    for node in fdt.all_compatible(COMPAT) {
        let base = {
            let reg = node.reg()?;
            reg.iter::<u64, u64>().flatten().next()?.address as usize
        };
        let reg_shift = node
            .properties()
            .find("reg-shift")
            .and_then(|prop| be_u32(prop.value))
            .unwrap_or(0);
        let reg_io_width = node
            .properties()
            .find("reg-io-width")
            .and_then(|prop| be_u32(prop.value))
            .unwrap_or(1);
        return Some(UartInfo {
            base,
            reg_shift,
            reg_io_width,
        });
    }
    None
}
