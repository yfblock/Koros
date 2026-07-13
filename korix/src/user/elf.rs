//! ELF loader for user processes.
//!
//! Parses ELF64 binaries, maps PT_LOAD segments into user address space,
//! and returns the entry point.

use alloc::vec::Vec;
use kor::{MapSize, MappingFlags};

/// ELF header (first 64 bytes of an ELF64 file)
#[repr(C)]
pub struct Elf64Ehdr {
    pub ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

/// ELF program header
#[repr(C)]
pub struct Elf64Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

pub const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

/// Base address for PIE executables (after clearing boot identity map).
const LOAD_BASE: usize = 0x0001_0000; // 64 KiB

const PAGE_SIZE: usize = 4096;

/// User stack top (256 GiB — well above any program).
const USER_STACK_TOP: usize = 0x10_0000_0000;
/// User stack size (64 KiB = 16 pages).
const USER_STACK_SIZE: usize = 64 * 1024;

/// Result of loading an ELF binary.
pub struct LoadedProgram {
    pub entry: usize,
    pub stack_top: usize,
}

/// Check if a buffer is a valid ELF64 binary.
pub fn is_elf(data: &[u8]) -> bool {
    data.len() >= 4 && &data[0..4] == b"\x7fELF"
}

/// Load an ELF64 binary into user address space.
///
/// Check if a virtual address is mapped to RAM (physical address >= 0x80000000).
/// Identity-mapped boot pages have low physical addresses (< 0x80000000) and
/// are not RAM — they must not be reused for user segments.
fn is_user_page(arch: &dyn kor::ArchProvider, vaddr: usize) -> bool {
    match arch.translate(vaddr) {
        Some(paddr) => paddr >= 0x8000_0000,
        None => false,
    }
}

/// Maps all PT_LOAD segments (with user permission) and sets up a user stack
/// with argc/argv/envp/auxv. Returns the entry point and stack pointer.
pub fn load_elf(data: &[u8]) -> Result<LoadedProgram, &'static str> {
    if !is_elf(data) {
        return Err("not an ELF file");
    }
    if data.len() < core::mem::size_of::<Elf64Ehdr>() {
        return Err("ELF header too small");
    }

    let ehdr = unsafe { &*(data.as_ptr() as *const Elf64Ehdr) };

    // Check ELF class (64-bit)
    if ehdr.ident[4] != 2 {
        return Err("not ELF64");
    }

    // ET_DYN (3) = PIE, needs a base address. ET_EXEC (2) = non-PIE, use addresses as-is.
    let load_base = match ehdr.e_type {
        3 => LOAD_BASE,  // ET_DYN — position-independent, add base
        2 => 0,          // ET_EXEC — absolute addresses
        _ => return Err("unsupported ELF type"),
    };

    let arch = kor::arch::current();
    let phdr_size = core::mem::size_of::<Elf64Phdr>();

    for i in 0..ehdr.e_phnum as usize {
        let offset = ehdr.e_phoff as usize + i * phdr_size;
        if offset + phdr_size > data.len() {
            continue;
        }
        let phdr = unsafe { &*(data[offset..].as_ptr() as *const Elf64Phdr) };

        if phdr.p_type != PT_LOAD {
            continue;
        }

        // Build mapping flags from ELF p_flags
        let mut flags = MappingFlags::U;
        if phdr.p_flags & PF_R != 0 {
            flags = flags | MappingFlags::R;
        }
        if phdr.p_flags & PF_W != 0 {
            flags = flags | MappingFlags::W;
        }
        if phdr.p_flags & PF_X != 0 {
            flags = flags | MappingFlags::X;
        }

        // Virtual address range
        let seg_vaddr = load_base + phdr.p_vaddr as usize;
        let seg_end = seg_vaddr + phdr.p_memsz as usize;
        let page_start = seg_vaddr & !(PAGE_SIZE - 1);
        let page_end = (seg_end + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

        // Map and fill each page
        for vaddr in (page_start..page_end).step_by(PAGE_SIZE) {
            // Check if this page was already mapped BY A PREVIOUS ELF SEGMENT
            // (has U bit set). Identity-mapped boot pages (no U bit) must NOT
            // be reused — their physical address is not RAM.
            let kvaddr = if arch.translate(vaddr).is_some() && is_user_page(arch, vaddr) {
                // Reuse existing user-mapped page (shared between segments)
                arch.phys_to_virt(arch.translate(vaddr).unwrap())
            } else {
                // Allocate and map a new page
                let paddr = kor_frame::alloc_page().ok_or("out of memory")?;
                match arch.map(vaddr, paddr, flags, MapSize::Page4K) {
                    Ok(()) => {}
                    Err(e) => {
                        kor::println!("[korix] map failed: vaddr={:#x} err={:?}", vaddr, e);
                        return Err("map failed");
                    }
                }
                let kv = arch.phys_to_virt(paddr);
                unsafe { core::ptr::write_bytes(kv as *mut u8, 0, PAGE_SIZE); }
                kv
            };

            // Copy file data that falls within this page.
            // The segment may start in the middle of this page (shared page).
            let file_start = phdr.p_offset as usize;
            let file_end = file_start + phdr.p_filesz as usize;
            let seg_start_in_page = seg_vaddr.saturating_sub(vaddr);
            let seg_off = vaddr.saturating_sub(seg_vaddr);
            // Copy file data that falls within this page.
            if seg_off < phdr.p_filesz as usize && seg_start_in_page < PAGE_SIZE {
                let copy_start = file_start + seg_off;
                let copy_len = core::cmp::min(
                    PAGE_SIZE - seg_start_in_page,
                    phdr.p_filesz as usize - seg_off,
                );
                let copy_end = copy_start + copy_len;
                if copy_start < copy_end && copy_end <= data.len() {
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            data[copy_start..copy_end].as_ptr(),
                            (kvaddr + seg_start_in_page) as *mut u8,
                            copy_len,
                        );
                    }
                }
            }
            // Zero BSS area (memsz > filesz) within this page.
            // BSS is the gap between file data end and memory end, limited to the page.
            let data_in_page = (phdr.p_filesz as usize)
                .saturating_sub(seg_off)
                .min(PAGE_SIZE - seg_start_in_page);
            let mem_in_page = (phdr.p_memsz as usize)
                .saturating_sub(seg_off)
                .min(PAGE_SIZE - seg_start_in_page);
            if data_in_page < mem_in_page {
                let bss_start = seg_start_in_page + data_in_page;
                let bss_end = seg_start_in_page + mem_in_page;
                unsafe {
                    core::ptr::write_bytes(
                        (kvaddr + bss_start) as *mut u8,
                        0,
                        bss_end - bss_start,
                    );
                }
            }
        }

        kor::println!(
            "[korix] ELF: LOAD vaddr={:#x} filesz={} memsz={} flags={:#x}",
            seg_vaddr, phdr.p_filesz, phdr.p_memsz, phdr.p_flags
        );
    }

    let entry = load_base + ehdr.e_entry as usize;
    // Compute the virtual address of the program headers.
    // e_phoff is a file offset; the VA is segment.vaddr + (e_phoff - segment.offset).
    let mut phdr_addr = load_base + ehdr.e_phoff as usize; // fallback
    for i in 0..ehdr.e_phnum as usize {
        let off = ehdr.e_phoff as usize + i * core::mem::size_of::<Elf64Phdr>();
        if off + core::mem::size_of::<Elf64Phdr>() > data.len() { continue; }
        let ph = unsafe { &*(data[off..].as_ptr() as *const Elf64Phdr) };
        if ph.p_type == PT_LOAD {
            let file_off = ehdr.e_phoff as usize;
            if file_off >= ph.p_offset as usize && file_off < ph.p_offset as usize + ph.p_filesz as usize {
                phdr_addr = load_base + ph.p_vaddr as usize + (file_off - ph.p_offset as usize);
                break;
            }
        }
    }
    let phnum = ehdr.e_phnum as usize;
    let stack_top = setup_user_stack(entry, phdr_addr, phnum);

    unsafe { core::arch::asm!("sfence.vma"); }

    Ok(LoadedProgram {
        entry,
        stack_top,
    })
}

/// Helper: write bytes to a user virtual address via the kernel direct map.
fn write_user(arch: &dyn kor::ArchProvider, uva: usize, data: &[u8]) {
    let paddr = arch.translate(uva).expect("write_user: unmapped");
    let kva = arch.phys_to_virt(paddr); // translate() includes page offset
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), kva as *mut u8, data.len()); }
}

/// Helper: write a usize to a user virtual address via the kernel direct map.
fn write_user_usize(arch: &dyn kor::ArchProvider, uva: usize, val: usize) {
    let paddr = arch.translate(uva).expect("write_user: unmapped");
    let kva = arch.phys_to_virt(paddr); // translate() includes page offset
    unsafe { *(kva as *mut usize) = val; }
}

/// Set up the user stack: allocate pages and populate argc/argv/envp/auxv.
/// `entry` is the program entry point, `phdr_addr` is the virtual address of
/// the program headers, `phnum` is the number of program headers.
fn setup_user_stack(entry: usize, phdr_addr: usize, phnum: usize) -> usize {
    let arch = kor::arch::current();
    let stack_base = USER_STACK_TOP - USER_STACK_SIZE;

    // Map stack pages (RW + U)
    let flags = MappingFlags::U | MappingFlags::R | MappingFlags::W;
    for vaddr in (stack_base..USER_STACK_TOP).step_by(PAGE_SIZE) {
        let paddr = kor_frame::alloc_page().expect("OOM: user stack");
        arch.map(vaddr, paddr, flags, MapSize::Page4K).expect("map: user stack");
        let kvaddr = arch.phys_to_virt(paddr);
        unsafe {
            core::ptr::write_bytes(kvaddr as *mut u8, 0, PAGE_SIZE);
        }
    }

    // Linux auxiliary vector constants
    const AT_NULL: usize = 0;
    const AT_PHDR: usize = 3;
    const AT_PHNUM: usize = 5;
    const AT_PAGESZ: usize = 6;
    const AT_BASE: usize = 7;
    const AT_ENTRY: usize = 9;
    const AT_RANDOM: usize = 25;

    let argv0 = b"/bin/sh\0";
    let env0 = b"PATH=/bin\0";
    let sp = USER_STACK_TOP;

    // Place strings at the top of the stack
    let random_off = sp - 16;
    let random_off = random_off & !0xF;
    let rand_bytes: [u8; 16] = [0x42; 16];
    write_user(arch, random_off, &rand_bytes);

    let env0_off = random_off - env0.len();
    let env0_off = env0_off & !0xF;
    write_user(arch, env0_off, env0);

    let str0_off = env0_off - argv0.len();
    let str0_off = str0_off & !0xF;
    write_user(arch, str0_off, argv0);

    // Build the argc/argv/envp/auxv array.
    // argc(1) + argv[0](1) + NULL(1) + envp[0](1) + NULL(1) + auxv(7*2=14) = 19 entries
    let array_size = 19 * 8;
    let array_top = str0_off;
    let array_base = array_top - array_size;
    let array_base = array_base & !0xF;

    let mut i = 0;
    // argc = 1
    write_user_usize(arch, array_base + i * 8, 1); i += 1;
    // argv[0] = "/bin/sh"
    write_user_usize(arch, array_base + i * 8, str0_off); i += 1;
    // argv NULL
    write_user_usize(arch, array_base + i * 8, 0); i += 1;
    // envp[0] = "PATH=/bin"
    write_user_usize(arch, array_base + i * 8, env0_off); i += 1;
    // envp NULL
    write_user_usize(arch, array_base + i * 8, 0); i += 1;
    // auxv entries (type, value pairs)
    write_user_usize(arch, array_base + i * 8, AT_PHDR);   i += 1;
    write_user_usize(arch, array_base + i * 8, phdr_addr);  i += 1;
    write_user_usize(arch, array_base + i * 8, AT_PHNUM);   i += 1;
    write_user_usize(arch, array_base + i * 8, phnum);      i += 1;
    write_user_usize(arch, array_base + i * 8, AT_PAGESZ);  i += 1;
    write_user_usize(arch, array_base + i * 8, 4096);       i += 1;
    write_user_usize(arch, array_base + i * 8, AT_BASE);    i += 1;
    write_user_usize(arch, array_base + i * 8, 0);          i += 1;
    write_user_usize(arch, array_base + i * 8, AT_ENTRY);   i += 1;
    write_user_usize(arch, array_base + i * 8, entry);      i += 1;
    write_user_usize(arch, array_base + i * 8, AT_RANDOM);  i += 1;
    write_user_usize(arch, array_base + i * 8, random_off); i += 1;
    write_user_usize(arch, array_base + i * 8, AT_NULL);    i += 1;
    write_user_usize(arch, array_base + i * 8, 0);          i += 1;

    kor::println!(
        "[korix] user stack: {:#x}-{:#x}, sp={:#x}",
        stack_base, USER_STACK_TOP, array_base
    );


    array_base
}
