#![allow(unsafe_op_in_unsafe_fn)]
use core::sync::atomic::{AtomicUsize, Ordering};

pub fn kernel_offset() -> usize {
    0xffff_8000_0000_0000
}

pub(crate) static MULTIBOOT_INFO: AtomicUsize = AtomicUsize::new(0);

pub fn set_multiboot_info(mbi: usize) {
    MULTIBOOT_INFO.store(mbi, Ordering::Relaxed);
}

/// Detect physical memory regions from the Multiboot memory map and add them
/// to the allocator.
pub fn init(alloc: &mut crate::mm::frame_allocator::FrameAllocator) {
    let mbi = MULTIBOOT_INFO.load(Ordering::Relaxed);
    if mbi != 0 {
        unsafe {
            parse_multiboot_regions(mbi, |start, end| {
                alloc.add_region(start, end);
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Multiboot1 memory-map parser
// ---------------------------------------------------------------------------

use core::mem;

const MULTIBOOT_INFO_MMAP: u32 = 1 << 6;

#[repr(C, packed)]
struct MultibootInfo {
    flags: u32,
    // … many fields we don't care about …
    mmap_len: u32,
    mmap_addr: u32,
    // … more fields …
}

#[repr(C, packed)]
struct MmapEntry {
    size: u32,
    addr_low: u32,
    addr_high: u32,
    len_low: u32,
    len_high: u32,
    type_: u32,
}

const MULTIBOOT_MEMORY_AVAILABLE: u32 = 1;

/// Parse the Multiboot memory map at `mbi_addr` and call `add_region` for
/// every available physical-memory region.
unsafe fn parse_multiboot_regions(
    mbi_addr: usize,
    mut add_region: impl FnMut(usize, usize),
) -> usize {
    let info = &*(mbi_addr as *const MultibootInfo);
    let flags = info.flags;

    if flags & MULTIBOOT_INFO_MMAP == 0 {
        return 0;
    }

    let mmap_addr = info.mmap_addr as usize;
    let mmap_len = info.mmap_len as usize;
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
