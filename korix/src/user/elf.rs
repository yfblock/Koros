//! ELF loader for user processes.
//!
//! Parses ELF64 binaries and maps segments into user address space.

use alloc::vec::Vec;

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

/// Check if a buffer is a valid ELF64 binary.
pub fn is_elf(data: &[u8]) -> bool {
    data.len() >= 4 && &data[0..4] == b"\x7fELF"
}

/// Load an ELF64 binary and return the entry point.
///
/// # Safety
/// The caller must ensure the ELF data is valid and the address space
/// is set up to receive the mappings.
pub fn load_elf(data: &[u8]) -> Result<u64, &'static str> {
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

    // Read program headers
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

        // TODO: map segment at phdr.p_vaddr with phdr.p_memsz
        // For now, just print info
        kor::println!(
            "[korix] ELF: LOAD vaddr={:#x} filesz={} memsz={}",
            phdr.p_vaddr, phdr.p_filesz, phdr.p_memsz
        );

        // Copy file content to virtual address (TODO: actual page table mapping)
        let file_data = &data[phdr.p_offset as usize..(phdr.p_offset + phdr.p_filesz) as usize];
        let _ = file_data; // TODO: copy to user space
    }

    Ok(ehdr.e_entry)
}
