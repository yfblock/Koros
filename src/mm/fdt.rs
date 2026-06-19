#![allow(unsafe_op_in_unsafe_fn)]
//! Minimal FDT (Flattened Device Tree) memory-region parser.
//!
//! Scans the tree for the `/memory` node and extracts its `reg` property,
//! yielding physical-memory ranges.  This is all we need for boot memory
//! detection on riscv64, aarch64, and loongarch64.
//!
//! FDT format: <https://devicetree-specification.readthedocs.io/>

const FDT_MAGIC: u32 = 0xD00DFEED;

const FDT_BEGIN_NODE: u32 = 0x01;
const FDT_END_NODE: u32   = 0x02;
const FDT_PROP: u32       = 0x03;
const FDT_NOP: u32        = 0x04;
const FDT_END: u32        = 0x09;

/// FDT header (big-endian, 40 bytes).
#[repr(C)]
struct FdtHeader {
    magic: u32,
    totalsize: u32,
    off_dt_struct: u32,
    off_dt_strings: u32,
    off_mem_rsvmap: u32,
    version: u32,
    last_comp_version: u32,
    boot_cpuid_phys: u32,
    size_dt_strings: u32,
    size_dt_struct: u32,
}

/// Parse FDT at `fdt_base` (physical address) and call `add_region` for each
/// physical-memory region found under `/memory`.
///
/// Returns the number of regions added.
pub unsafe fn parse_memory_regions(
    fdt_base: usize,
    mut add_region: impl FnMut(usize, usize),
) -> usize {
    let hdr = &*(fdt_base as *const FdtHeader);
    let magic = u32::from_be(hdr.magic);
    if magic != FDT_MAGIC {
        return 0;
    }

    let struct_off = u32::from_be(hdr.off_dt_struct) as usize;
    let strings_off = u32::from_be(hdr.off_dt_strings) as usize;

    let struct_ptr = (fdt_base + struct_off) as *const u8;
    let strings_ptr = (fdt_base + strings_off) as *const u8;

    let mut depth = 0usize;
    let mut in_memory = false;
    let mut count = 0usize;
    let mut pos = 0usize;

    loop {
        let token = u32::from_be(read_be32(struct_ptr.add(pos)));
        pos += 4;

        match token {
            FDT_BEGIN_NODE => {
                // Node name follows as a null-terminated string, padded to 4 bytes.
                let name_ptr = struct_ptr.add(pos);
                let name = cstr(name_ptr);
                // Round up to 4-byte boundary (including NUL).
                let name_len = name.len() + 1; // include NUL
                pos += (name_len + 3) & !3;

                depth += 1;
                if depth == 1 && name == "memory" {
                    in_memory = true;
                }
            }

            FDT_END_NODE => {
                depth -= 1;
                in_memory = false;
            }

            FDT_PROP => {
                // Property: len(4) + nameoff(4) + value(len, padded to 4)
                let prop_len = u32::from_be(read_be32(struct_ptr.add(pos))) as usize;
                let name_off = u32::from_be(read_be32(struct_ptr.add(pos + 4))) as usize;
                let val_ptr = struct_ptr.add(pos + 8);
                // Round value up to 4 bytes.
                let val_padded = (prop_len + 3) & !3;
                pos += 8 + val_padded;

                if in_memory {
                    let name_ptr = strings_ptr.add(name_off);
                    let prop_name = cstr(name_ptr);
                    if prop_name == "reg" {
                        // reg: array of (address, size) pairs.
                        // #address-cells and #size-cells default to 2 on 64-bit.
                        // All 64-bit QEMU virt machines use (2, 2).
                        const ADDR_CELLS: usize = 2;
                        const SIZE_CELLS: usize = 2;
                        const CELL: usize = 4; // bytes per cell

                        let cell_size = ADDR_CELLS + SIZE_CELLS;
                        let pair_bytes = cell_size * CELL;
                        let mut off = 0usize;
                        while off + pair_bytes <= prop_len {
                            let addr = if ADDR_CELLS == 2 {
                                (read_be32(val_ptr.add(off)) as u64) << 32
                                    | read_be32(val_ptr.add(off + 4)) as u64
                            } else {
                                read_be32(val_ptr.add(off)) as u64
                            };
                            let size = if SIZE_CELLS == 2 {
                                (read_be32(val_ptr.add(off + ADDR_CELLS * 4)) as u64) << 32
                                    | read_be32(val_ptr.add(off + ADDR_CELLS * 4 + 4)) as u64
                            } else {
                                read_be32(val_ptr.add(off + ADDR_CELLS * 4)) as u64
                            };

                            if size > 0 {
                                add_region(addr as usize, (addr + size) as usize);
                                count += 1;
                            }
                            off += pair_bytes;
                        }
                    }
                }
            }

            FDT_NOP => {}

            FDT_END => break,

            _ => break,
        }
    }

    count
}

/// Read a big-endian u32 from a possibly-unaligned pointer.
unsafe fn read_be32(p: *const u8) -> u32 {
    u32::from_be_bytes([
        *p,
        *p.add(1),
        *p.add(2),
        *p.add(3),
    ])
}

/// Return the string up to the first NUL (not including NUL).
fn cstr<'a>(p: *const u8) -> &'a str {
    let mut len = 0;
    unsafe {
        while *p.add(len) != 0 {
            len += 1;
        }
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(p, len))
    }
}
