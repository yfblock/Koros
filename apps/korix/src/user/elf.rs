//! ELF loader for user processes.
//!
//! Parses ELF64 binaries, maps PT_LOAD segments into user address space,
//! and returns the entry point.

use alloc::string::String;
use alloc::vec::Vec;
use kor_arch::MappingFlags;

use super::layout::{
    ADDRESS_LIMIT, INTERP_BASE, LOAD_BASE, PAGE_SIZE, STACK_SIZE as USER_STACK_SIZE,
    STACK_TOP as USER_STACK_TOP,
};
use crate::mm::AddressSpace;

use super::elf_format::{self, ELF_HEADER_SIZE, ElfHeader, PROGRAM_HEADER_SIZE};

pub const PT_LOAD: u32 = 1;
const PT_INTERP: u32 = 3;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;
const ELFCLASS64: u8 = 2;
const EM_X86_64: u16 = 62;
const EM_AARCH64: u16 = 183;
const EM_RISCV: u16 = 243;

fn expected_machine() -> u16 {
    if cfg!(target_arch = "riscv64") {
        EM_RISCV
    } else if cfg!(target_arch = "aarch64") {
        EM_AARCH64
    } else {
        EM_X86_64
    }
}
pub struct LoadedProgram {
    pub entry: usize,
    pub stack_top: usize,
}

#[derive(Clone, Copy)]
struct AuxImage {
    entry: usize,
    phdr_addr: usize,
    phnum: usize,
}
/// Check if a buffer is a valid ELF64 binary.
pub fn is_elf(data: &[u8]) -> bool {
    data.len() >= 4 && &data[0..4] == b"\x7fELF"
}
/// Return the `PT_INTERP` path requested by an ELF image, if any.
pub fn interpreter_path(data: &[u8]) -> Result<Option<String>, &'static str> {
    let header = elf_format::header(data).ok_or("ELF header too small")?;
    for index in 0..header.program_count as usize {
        let offset = (header.program_offset as usize)
            .checked_add(
                index
                    .checked_mul(PROGRAM_HEADER_SIZE)
                    .ok_or("program header overflow")?,
            )
            .ok_or("program header overflow")?;
        let phdr =
            elf_format::program_header(data, offset).ok_or("program header outside image")?;
        if phdr.segment_type != PT_INTERP {
            continue;
        }
        let start = phdr.offset as usize;
        let end = start
            .checked_add(phdr.file_size as usize)
            .ok_or("interpreter path overflow")?;
        let path = data
            .get(start..end)
            .ok_or("interpreter path outside image")?;
        let path = path.strip_suffix(&[0]).unwrap_or(path);
        return core::str::from_utf8(path)
            .map(String::from)
            .map(Some)
            .map_err(|_| "invalid interpreter path");
    }
    Ok(None)
}
/// Load a complete program, including its optional runtime interpreter.
pub fn load_program(
    address_space: &mut AddressSpace,
    data: &[u8],
    interpreter: Option<&[u8]>,
    argv: &[String],
    env: &[String],
) -> Result<LoadedProgram, &'static str> {
    let main = map_elf(address_space, data, None)?;
    let entry = if let Some(interpreter) = interpreter {
        map_elf(address_space, interpreter, Some(INTERP_BASE))?.entry
    } else {
        main.entry
    };
    let stack_top = setup_user_stack(
        address_space,
        main.entry,
        main.phdr_addr,
        main.phnum,
        if interpreter.is_some() {
            INTERP_BASE
        } else {
            0
        },
        argv,
        env,
    )?;
    Ok(LoadedProgram { entry, stack_top })
}

/// Load an ELF image without a runtime interpreter.
#[expect(dead_code, reason = "staged: pending wiring")]
pub fn load_elf(
    address_space: &mut AddressSpace,
    data: &[u8],
    argv: &[String],
) -> Result<LoadedProgram, &'static str> {
    load_program(
        address_space,
        data,
        None,
        argv,
        &[alloc::string::String::from(
            "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        )],
    )
}

fn map_elf(
    address_space: &mut AddressSpace,
    data: &[u8],
    forced_base: Option<usize>,
) -> Result<AuxImage, &'static str> {
    if !is_elf(data) {
        return Err("not an ELF file");
    }
    let header = elf_format::header(data).ok_or("ELF header too small")?;
    if header.ident[4] != ELFCLASS64 || header.machine != expected_machine() {
        return Err("wrong ELF architecture");
    }
    if header.header_size as usize != ELF_HEADER_SIZE
        || header.program_entry_size as usize != PROGRAM_HEADER_SIZE
    {
        return Err("invalid ELF header sizes");
    }
    let load_base = forced_base.unwrap_or(match header.elf_type {
        3 => LOAD_BASE,
        2 => 0,
        _ => return Err("unsupported ELF type"),
    });
    let program_header_end = (header.program_offset as usize)
        .checked_add(
            (header.program_count as usize)
                .checked_mul(PROGRAM_HEADER_SIZE)
                .ok_or("program header overflow")?,
        )
        .ok_or("program header overflow")?;
    if program_header_end > data.len() {
        return Err("program headers outside image");
    }
    load_active(address_space, data, header, load_base)
}

fn load_active(
    address_space: &mut AddressSpace,
    data: &[u8],
    header: ElfHeader,
    load_base: usize,
) -> Result<AuxImage, &'static str> {
    let mut page_flags = alloc::collections::BTreeMap::new();
    for index in 0..header.program_count as usize {
        let offset = header.program_offset as usize + index * PROGRAM_HEADER_SIZE;
        let phdr =
            elf_format::program_header(data, offset).ok_or("program header outside image")?;
        if phdr.segment_type != PT_LOAD {
            continue;
        }
        let mut flags = MappingFlags::U;
        if phdr.flags & PF_R != 0 {
            flags = flags | MappingFlags::R;
        }
        if phdr.flags & PF_W != 0 {
            flags = flags | MappingFlags::W;
        }
        if phdr.flags & PF_X != 0 {
            flags = flags | MappingFlags::X;
        }
        let start = (load_base + phdr.virtual_address as usize) & !(PAGE_SIZE - 1);
        let end =
            (load_base + phdr.virtual_address as usize + phdr.memory_size as usize + PAGE_SIZE - 1)
                & !(PAGE_SIZE - 1);
        for page in (start..end).step_by(PAGE_SIZE) {
            page_flags
                .entry(page)
                .and_modify(|combined| *combined = *combined | flags)
                .or_insert(flags);
        }
    }

    for index in 0..header.program_count as usize {
        let offset = header.program_offset as usize + index * PROGRAM_HEADER_SIZE;
        let phdr =
            elf_format::program_header(data, offset).ok_or("program header outside image")?;
        if phdr.segment_type != PT_LOAD {
            continue;
        }
        let mut flags = MappingFlags::U;
        if phdr.flags & PF_R != 0 {
            flags = flags | MappingFlags::R;
        }
        if phdr.flags & PF_W != 0 {
            flags = flags | MappingFlags::W;
        }
        if phdr.flags & PF_X != 0 {
            flags = flags | MappingFlags::X;
        }
        let seg_vaddr = load_base
            .checked_add(phdr.virtual_address as usize)
            .ok_or("segment address overflow")?;
        let seg_end = seg_vaddr
            .checked_add(phdr.memory_size as usize)
            .ok_or("segment address overflow")?;
        if phdr.file_size > phdr.memory_size {
            return Err("segment file size exceeds memory size");
        }
        let file_end = (phdr.offset as usize)
            .checked_add(phdr.file_size as usize)
            .ok_or("segment file overflow")?;
        if file_end > data.len() || seg_vaddr >= ADDRESS_LIMIT || seg_end > ADDRESS_LIMIT {
            return Err("segment outside user address space");
        }
        let page_start = seg_vaddr & !(PAGE_SIZE - 1);
        let page_end = seg_end
            .checked_add(PAGE_SIZE - 1)
            .ok_or("segment address overflow")?
            & !(PAGE_SIZE - 1);

        // Map and fill each page
        for vaddr in (page_start..page_end).step_by(PAGE_SIZE) {
            // Reuse only pages recorded in this process's authoritative VM
            // map. A translation inherited from the kernel root is never a
            // valid ELF page.
            let kvaddr = if let Some(paddr) = address_space.translate(vaddr) {
                kor_arch::phys_to_virt(paddr)
            } else {
                let combined_flags = page_flags.get(&vaddr).copied().unwrap_or(flags);
                let paddr = address_space
                    .map_zeroed(vaddr, combined_flags)
                    .map_err(|_| "map failed")?;
                kor_arch::phys_to_virt(paddr)
            };

            // The segment may start in the middle of this page (shared page).
            let file_start = phdr.offset as usize;
            let seg_start_in_page = seg_vaddr.saturating_sub(vaddr);
            let seg_off = vaddr.saturating_sub(seg_vaddr);
            if seg_off < phdr.file_size as usize && seg_start_in_page < PAGE_SIZE {
                let copy_start = file_start + seg_off;
                let copy_len = core::cmp::min(
                    PAGE_SIZE - seg_start_in_page,
                    phdr.file_size as usize - seg_off,
                );
                let copy_end = copy_start + copy_len;
                if copy_start < copy_end && copy_end <= data.len() {
                    // SAFETY: `map_zeroed` or the address-space lookup above
                    // yields a writable direct-map page. The checked segment
                    // bounds restrict source and destination to `copy_len`.
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
            let data_in_page = (phdr.file_size as usize)
                .saturating_sub(seg_off)
                .min(PAGE_SIZE - seg_start_in_page);
            let mem_in_page = (phdr.memory_size as usize)
                .saturating_sub(seg_off)
                .min(PAGE_SIZE - seg_start_in_page);
            if data_in_page < mem_in_page {
                let bss_start = seg_start_in_page + data_in_page;
                let bss_end = seg_start_in_page + mem_in_page;
                // SAFETY: the destination is the mapped page above and the
                // calculated BSS interval is bounded by that page.
                unsafe {
                    core::ptr::write_bytes((kvaddr + bss_start) as *mut u8, 0, bss_end - bss_start);
                }
            }
        }
    }

    let entry = load_base + header.entry as usize;
    let mut phdr_addr = load_base + header.program_offset as usize;
    for index in 0..header.program_count as usize {
        let offset = header.program_offset as usize + index * PROGRAM_HEADER_SIZE;
        let phdr =
            elf_format::program_header(data, offset).ok_or("program header outside image")?;
        if phdr.segment_type == PT_LOAD {
            let file_offset = header.program_offset as usize;
            if file_offset >= phdr.offset as usize
                && file_offset < phdr.offset as usize + phdr.file_size as usize
            {
                phdr_addr = load_base
                    + phdr.virtual_address as usize
                    + (file_offset - phdr.offset as usize);
                break;
            }
        }
    }
    Ok(AuxImage {
        entry,
        phdr_addr,
        phnum: header.program_count as usize,
    })
}
/// Set up the Linux argc/argv/envp/auxv stack in `address_space`.
fn setup_user_stack(
    address_space: &mut AddressSpace,
    entry: usize,
    phdr_addr: usize,
    phnum: usize,
    interpreter_base: usize,
    argv: &[String],
    env: &[String],
) -> Result<usize, &'static str> {
    let stack_base = USER_STACK_TOP - USER_STACK_SIZE;
    let flags = MappingFlags::U | MappingFlags::R | MappingFlags::W;
    for vaddr in (stack_base..USER_STACK_TOP).step_by(PAGE_SIZE) {
        address_space
            .map_zeroed(vaddr, flags)
            .map_err(|_| "map user stack failed")?;
    }

    const AT_NULL: usize = 0;
    const AT_PHDR: usize = 3;
    const AT_PHNUM: usize = 5;
    const AT_PAGESZ: usize = 6;
    const AT_BASE: usize = 7;
    const AT_ENTRY: usize = 9;
    const AT_RANDOM: usize = 25;

    const AT_PHENT: usize = 4;
    const AT_UID: usize = 11;
    const AT_EUID: usize = 12;
    const AT_GID: usize = 13;
    const AT_EGID: usize = 14;
    const AT_SECURE: usize = 23;
    const AT_EXECFN: usize = 31;
    let mut sp = USER_STACK_TOP;
    sp = (sp - 16) & !0xF;
    let random_off = sp;
    address_space
        .write(random_off, &[0x42; 16])
        .map_err(|_| "write random")?;

    let mut env_offsets = Vec::with_capacity(env.len());
    for variable in env.iter().rev() {
        let bytes = variable.as_bytes();
        sp = (sp - bytes.len() - 1) & !0xF;
        address_space
            .write(sp, bytes)
            .map_err(|_| "write environment")?;
        address_space
            .write(sp + bytes.len(), &[0])
            .map_err(|_| "terminate environment")?;
        env_offsets.push(sp);
    }
    env_offsets.reverse();

    let mut arg_offsets = Vec::with_capacity(argv.len());
    for arg in argv.iter().rev() {
        let bytes = arg.as_bytes();
        sp = (sp - bytes.len() - 1) & !0xF;
        address_space
            .write(sp, bytes)
            .map_err(|_| "write argument")?;
        address_space
            .write(sp + bytes.len(), &[0])
            .map_err(|_| "terminate argument")?;
        arg_offsets.push(sp);
    }
    arg_offsets.reverse();
    let execfn = arg_offsets.first().copied().unwrap_or(0);
    let mut words = Vec::new();
    words.push(arg_offsets.len());
    words.extend_from_slice(&arg_offsets);
    words.push(0);
    words.extend_from_slice(&env_offsets);
    words.push(0);
    words.extend_from_slice(&[
        AT_PHDR,
        phdr_addr,
        AT_PHENT,
        PROGRAM_HEADER_SIZE,
        AT_PHNUM,
        phnum,
        AT_PAGESZ,
        PAGE_SIZE,
        AT_BASE,
        interpreter_base,
        AT_ENTRY,
        entry,
        AT_UID,
        0,
        AT_EUID,
        0,
        AT_GID,
        0,
        AT_EGID,
        0,
        AT_SECURE,
        0,
        AT_RANDOM,
        random_off,
        AT_EXECFN,
        execfn,
        AT_NULL,
        0,
    ]);
    let array_size = words.len() * core::mem::size_of::<usize>();
    let array_base = (sp - array_size) & !0xF;
    let mut bytes = Vec::with_capacity(array_size);
    for word in words {
        bytes.extend_from_slice(&word.to_ne_bytes());
    }
    address_space
        .write(array_base, &bytes)
        .map_err(|_| "write initial stack")?;
    Ok(array_base)
}
