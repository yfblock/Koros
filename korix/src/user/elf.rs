//! ELF loader for user processes.
//!
//! Parses ELF64 binaries, maps PT_LOAD segments into user address space,
//! and returns the entry point.

use alloc::string::String;
use alloc::vec::Vec;
use kor::{MapSize, MappingFlags};

use crate::mm::AddressSpace;

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
// Above identity-mapped region (512 GiB on aarch64, 4 GiB on riscv64)
    #[cfg(target_arch = "riscv64")]
    const USER_STACK_TOP: usize = 0x10_0000_0000;
    #[cfg(target_arch = "aarch64")]
    const USER_STACK_TOP: usize = 0x80_0000_0000;
    #[cfg(target_arch = "x86_64")]
    const USER_STACK_TOP: usize = 0x8_0000_0000; // Above 4 GiB identity map
const USER_STACK_SIZE: usize = 64 * 1024;

pub struct LoadedProgram {
    pub entry: usize,
    pub stack_top: usize,
}
/// Check if a buffer is a valid ELF64 binary.
pub fn is_elf(data: &[u8]) -> bool {
    data.len() >= 4 && &data[0..4] == b"\x7fELF"
}

/// Load an ELF image into `address_space` without replacing the caller's
/// currently active image.
pub fn load_elf(
    address_space: &AddressSpace,
    data: &[u8],
    argv: &[String],
) -> Result<LoadedProgram, &'static str> {
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

    address_space.with_active(|| load_active(address_space, data, argv, ehdr, load_base))
}

fn load_active(
    address_space: &AddressSpace,
    data: &[u8],
    argv: &[String],
    ehdr: &Elf64Ehdr,
    load_base: usize,
) -> Result<LoadedProgram, &'static str> {
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
            let kvaddr = if let Some(paddr) = arch.translate(vaddr) {
                // Reuse existing user-mapped page (shared between segments)
                arch.phys_to_virt(paddr)
            } else {
                // Allocate and map a new page
                let paddr = kor_frame::alloc_page().ok_or("out of memory")?;
                if arch.map(vaddr, paddr, flags, MapSize::Page4K).is_err() {
                    kor_frame::free_frames(paddr, 1);
                    return Err("map failed");
                }
                let kv = arch.phys_to_virt(paddr);
                unsafe { core::ptr::write_bytes(kv as *mut u8, 0, PAGE_SIZE); }
                kv
            };

            // Copy file data that falls within this page.
            // The segment may start in the middle of this page (shared page).
            let file_start = phdr.p_offset as usize;
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
    let stack_top = setup_user_stack(address_space, entry, phdr_addr, phnum, argv)?;

    Ok(LoadedProgram {
        entry,
        stack_top,
    })
}
/// Set up the Linux argc/argv/envp/auxv stack in `address_space`.
fn setup_user_stack(
    address_space: &AddressSpace,
    entry: usize,
    phdr_addr: usize,
    phnum: usize,
    argv: &[String],
) -> Result<usize, &'static str> {
    let stack_base = USER_STACK_TOP - USER_STACK_SIZE;
    let flags = MappingFlags::U | MappingFlags::R | MappingFlags::W;
    for vaddr in (stack_base..USER_STACK_TOP).step_by(PAGE_SIZE) {
        address_space.map_zeroed(vaddr, flags).map_err(|_| "map user stack failed")?;
    }

    const AT_NULL: usize = 0;
    const AT_PHDR: usize = 3;
    const AT_PHNUM: usize = 5;
    const AT_PAGESZ: usize = 6;
    const AT_BASE: usize = 7;
    const AT_ENTRY: usize = 9;
    const AT_RANDOM: usize = 25;

    let env0 = b"PATH=/bin\0";
    let mut sp = USER_STACK_TOP;
    sp = (sp - 16) & !0xF;
    let random_off = sp;
    address_space.write(random_off, &[0x42; 16]).map_err(|_| "write random")?;
    sp = (sp - env0.len()) & !0xF;
    let env0_off = sp;
    address_space.write(env0_off, env0).map_err(|_| "write environment")?;

    let mut arg_offsets = Vec::with_capacity(argv.len());
    for arg in argv.iter().rev() {
        let bytes = arg.as_bytes();
        sp = (sp - bytes.len() - 1) & !0xF;
        address_space.write(sp, bytes).map_err(|_| "write argument")?;
        address_space.write(sp + bytes.len(), &[0]).map_err(|_| "terminate argument")?;
        arg_offsets.push(sp);
    }
    arg_offsets.reverse();

    let word_size = core::mem::size_of::<usize>();
    let array_size = (1 + arg_offsets.len() + 1 + 2 + 14) * word_size;
    let array_base = (sp - array_size) & !0xF;
    let mut words = Vec::with_capacity(array_size / word_size);
    words.push(arg_offsets.len());
    words.extend_from_slice(&arg_offsets);
    words.push(0);
    words.push(env0_off);
    words.push(0);
    words.extend_from_slice(&[
        AT_PHDR, phdr_addr,
        AT_PHNUM, phnum,
        AT_PAGESZ, PAGE_SIZE,
        AT_BASE, 0,
        AT_ENTRY, entry,
        AT_RANDOM, random_off,
        AT_NULL, 0,
    ]);
    let bytes = unsafe {
        core::slice::from_raw_parts(words.as_ptr() as *const u8, words.len() * word_size)
    };
    address_space.write(array_base, bytes).map_err(|_| "write initial stack")?;
    Ok(array_base)
}
