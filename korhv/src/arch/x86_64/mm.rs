//! x86_64 host memory helpers: kernel offset, multiboot info capture, and
//! physical memory detection via the Multiboot1 memory map.

use alloc::string::String;
use core::sync::atomic::{AtomicUsize, Ordering};

const KERNEL_OFFSET: usize = 0xffff_8000_0000_0000;

pub fn kernel_offset() -> usize {
    KERNEL_OFFSET
}

pub fn phys_to_virt(pa: usize) -> usize {
    pa + KERNEL_OFFSET
}

pub fn virt_to_phys(va: usize) -> usize {
    va - KERNEL_OFFSET
}

pub(crate) static MBI: AtomicUsize = AtomicUsize::new(0);

pub fn set_multiboot_info(mbi: usize) {
    MBI.store(mbi, Ordering::Relaxed);
}

pub fn multiboot_info() -> usize {
    MBI.load(Ordering::Relaxed)
}

pub fn dtb_ptr() -> usize {
    0
}

/// Read a u32 at a byte offset from a base address (volatile).
unsafe fn r32(base: usize, off: usize) -> u32 {
    unsafe { ((base + off) as *const u32).read_volatile() }
}

/// Read the kernel command line from the Multiboot1 info structure.
pub fn boot_cmdline() -> Option<String> {
    let mbi = MBI.load(Ordering::Relaxed);
    if mbi == 0 {
        return None;
    }
    // SAFETY: mbi points to the Multiboot1 info structure in identity-mapped
    // low memory.
    let flags = unsafe { r32(mbi, 0) };
    if flags & (1 << 2) == 0 {
        return None; // cmdline not valid
    }
    let cmdline_pa = unsafe { r32(mbi, 16) } as usize;
    if cmdline_pa == 0 {
        return None;
    }
    // SAFETY: the cmdline C string lives in low identity-mapped memory.
    let cstr: &[u8] = unsafe { core::slice::from_raw_parts(cmdline_pa as *const u8, 256) };
    let len = cstr.iter().position(|&b| b == 0).unwrap_or(cstr.len());
    String::from_utf8(cstr[..len].to_vec()).ok()
}

/// Multiboot1 memory-map entry (packed): size, base_addr, length, type.
#[repr(C, packed)]
struct MmapEntry {
    size: u32,
    base: u64,
    length: u64,
    mtype: u32,
}

/// Walk the Multiboot1 mmap and report available RAM via add_region.
pub fn detect_memory_regions(mut add_region: impl FnMut(usize, usize)) {
    let mbi = MBI.load(Ordering::Relaxed);
    if mbi == 0 {
        return;
    }
    // SAFETY: mbi points to the Multiboot1 info structure.
    let flags = unsafe { r32(mbi, 0) };
    if flags & (1 << 6) == 0 {
        // No mmap; fall back to mem_lower/mem_upper (bit 0).
        let mem_lower = unsafe { r32(mbi, 4) } as usize;
        let mem_upper = unsafe { r32(mbi, 8) } as usize;
        add_region(0, mem_lower * 1024);
        add_region(0x100000, 0x100000 + mem_upper * 1024);
        return;
    }
    // Multiboot1 info: mmap_length at byte offset 44, mmap_addr at 48.
    let mmap_len = unsafe { r32(mbi, 44) } as usize;
    let mmap_addr = unsafe { r32(mbi, 48) } as usize;
    let mut off = 0;
    while off + core::mem::size_of::<MmapEntry>() <= mmap_len {
        // SAFETY: entries live in low identity-mapped memory.
        let entry = unsafe { &*((mmap_addr + off) as *const MmapEntry) };
        let size = unsafe { core::ptr::addr_of!((*entry).size).read_unaligned() } as usize;
        let mtype = unsafe { core::ptr::addr_of!((*entry).mtype).read_unaligned() };
        let base = unsafe { core::ptr::addr_of!((*entry).base).read_unaligned() } as usize;
        let length = unsafe { core::ptr::addr_of!((*entry).length).read_unaligned() } as usize;
        if mtype == 1 {
            add_region(base, base + length);
        }
        off += size + 4;
    }
}
