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
