#![allow(unsafe_op_in_unsafe_fn)]
use alloc::string::String;
use core::sync::atomic::{AtomicUsize, Ordering};

pub fn kernel_offset() -> usize {
    0xffff_8000_0000_0000
}

/// x86_64 has no device tree (it uses Multiboot + PCI/ACPI).
pub fn dtb_ptr() -> usize {
    0
}

/// Read the kernel command line from the Multiboot information structure.
pub fn boot_cmdline() -> Option<String> {
    const MULTIBOOT_INFO_CMDLINE: u32 = 1 << 2;
    const MBI_CMDLINE: usize = 16;

    let mbi = MULTIBOOT_INFO.load(Ordering::Relaxed);
    if mbi == 0 {
        return None;
    }
    unsafe {
        let base = phys_to_virt(mbi) as *const u8;
        let flags = core::ptr::read_unaligned(base.add(MBI_FLAGS) as *const u32);
        if flags & MULTIBOOT_INFO_CMDLINE == 0 {
            return None;
        }
        let cmdline_phys =
            core::ptr::read_unaligned(base.add(MBI_CMDLINE) as *const u32) as usize;
        if cmdline_phys == 0 {
            return None;
        }
        let ptr = phys_to_virt(cmdline_phys) as *const u8;
        let mut len = 0usize;
        while core::ptr::read(ptr.add(len)) != 0 && len < 4096 {
            len += 1;
        }
        let bytes = core::slice::from_raw_parts(ptr, len);
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
}

pub fn phys_to_virt(pa: usize) -> usize {
    pa + kernel_offset()
}

pub fn virt_to_phys(va: usize) -> usize {
    va - kernel_offset()
}

pub(crate) static MULTIBOOT_INFO: AtomicUsize = AtomicUsize::new(0);

pub fn set_multiboot_info(mbi: usize) {
    MULTIBOOT_INFO.store(mbi, Ordering::Relaxed);
}

/// Nothing below the linked kernel image needs a fixed firmware carve-out on x86_64.
pub fn firmware_phys_start() -> usize {
    0
}

/// Detect physical memory regions from the Multiboot memory map and register
/// them with `add_region`.
pub fn init(mut add_region: impl FnMut(usize, usize)) {
    let mbi = MULTIBOOT_INFO.load(Ordering::Relaxed);
    if mbi != 0 {
        unsafe {
            parse_multiboot_regions(phys_to_virt(mbi), &mut add_region);
        }
    }
}

// ---------------------------------------------------------------------------
// Multiboot1 memory-map parser
// ---------------------------------------------------------------------------

use core::mem;

const MULTIBOOT_INFO_MMAP: u32 = 1 << 6;
const MULTIBOOT_MEMORY_AVAILABLE: u32 = 1;

const MBI_FLAGS: usize = 0;
const MBI_MMAP_LEN: usize = 44;
const MBI_MMAP_ADDR: usize = 48;

#[repr(C, packed)]
struct MmapEntry {
    size: u32,
    addr_low: u32,
    addr_high: u32,
    len_low: u32,
    len_high: u32,
    type_: u32,
}

/// Parse the Multiboot memory map at `mbi_va` (virtual) and call `add_region`.
unsafe fn parse_multiboot_regions(
    mbi_va: usize,
    mut add_region: impl FnMut(usize, usize),
) -> usize {
    let base = mbi_va as *const u8;
    let flags = core::ptr::read_unaligned(base.add(MBI_FLAGS) as *const u32);

    if flags & MULTIBOOT_INFO_MMAP == 0 {
        if flags & 1 != 0 {
            let mem_upper_kb =
                core::ptr::read_unaligned(base.add(8) as *const u32) as usize;
            let end = mem_upper_kb * 1024;
            if end > 0x100000 {
                add_region(0x100000, end);
                return 1;
            }
        }
        return 0;
    }

    let mmap_len = core::ptr::read_unaligned(base.add(MBI_MMAP_LEN) as *const u32) as usize;
    let mmap_phys = core::ptr::read_unaligned(base.add(MBI_MMAP_ADDR) as *const u32) as usize;
    let mmap_addr = phys_to_virt(mmap_phys);
    if mmap_len == 0 {
        return 0;
    }

    let mut count = 0usize;
    let mut offset = 0usize;

    while offset + mem::size_of::<MmapEntry>() <= mmap_len {
        let entry = &*((mmap_addr + offset) as *const MmapEntry);

        let base = (entry.addr_high as u64) << 32 | entry.addr_low as u64;
        let len = (entry.len_high as u64) << 32 | entry.len_low as u64;
        let typ = entry.type_;

        if typ == MULTIBOOT_MEMORY_AVAILABLE && len > 0 {
            let start = base as usize;
            let end = start.wrapping_add(len as usize);
            if end > 0x100000 {
                let real_start = if start < 0x100000 { 0x100000 } else { start };
                if end > real_start {
                    add_region(real_start, end);
                    count += 1;
                }
            }
        }

        offset += entry.size as usize + 4;
    }

    count
}
