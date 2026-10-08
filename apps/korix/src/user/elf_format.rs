//! Bounds-checked ELF64 header decoding.

use core::mem::{offset_of, size_of};

/// ELF64 file header fields consumed by the loader.
#[derive(Clone, Copy)]
pub(super) struct ElfHeader {
    pub ident: [u8; 16],
    pub elf_type: u16,
    pub machine: u16,
    pub entry: u64,
    pub program_offset: u64,
    pub header_size: u16,
    pub program_entry_size: u16,
    pub program_count: u16,
}

/// ELF64 program-header fields consumed by the loader.
#[derive(Clone, Copy)]
pub(super) struct ProgramHeader {
    pub segment_type: u32,
    pub flags: u32,
    pub offset: u64,
    pub virtual_address: u64,
    pub file_size: u64,
    pub memory_size: u64,
}

#[repr(C)]
struct RawElfHeader {
    ident: [u8; 16],
    elf_type: u16,
    machine: u16,
    version: u32,
    entry: u64,
    program_offset: u64,
    section_offset: u64,
    flags: u32,
    header_size: u16,
    program_entry_size: u16,
    program_count: u16,
    section_entry_size: u16,
    section_count: u16,
    section_names: u16,
}

#[repr(C)]
struct RawProgramHeader {
    segment_type: u32,
    flags: u32,
    offset: u64,
    virtual_address: u64,
    physical_address: u64,
    file_size: u64,
    memory_size: u64,
    alignment: u64,
}

pub(super) const ELF_HEADER_SIZE: usize = size_of::<RawElfHeader>();
pub(super) const PROGRAM_HEADER_SIZE: usize = size_of::<RawProgramHeader>();

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        data.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_u64(data: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        data.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

pub(super) fn header(data: &[u8]) -> Option<ElfHeader> {
    let ident = data.get(..16)?.try_into().ok()?;
    Some(ElfHeader {
        ident,
        elf_type: read_u16(data, offset_of!(RawElfHeader, elf_type))?,
        machine: read_u16(data, offset_of!(RawElfHeader, machine))?,
        entry: read_u64(data, offset_of!(RawElfHeader, entry))?,
        program_offset: read_u64(data, offset_of!(RawElfHeader, program_offset))?,
        header_size: read_u16(data, offset_of!(RawElfHeader, header_size))?,
        program_entry_size: read_u16(data, offset_of!(RawElfHeader, program_entry_size))?,
        program_count: read_u16(data, offset_of!(RawElfHeader, program_count))?,
    })
}

pub(super) fn program_header(data: &[u8], offset: usize) -> Option<ProgramHeader> {
    let data = data.get(offset..offset.checked_add(PROGRAM_HEADER_SIZE)?)?;
    Some(ProgramHeader {
        segment_type: read_u32(data, offset_of!(RawProgramHeader, segment_type))?,
        flags: read_u32(data, offset_of!(RawProgramHeader, flags))?,
        offset: read_u64(data, offset_of!(RawProgramHeader, offset))?,
        virtual_address: read_u64(data, offset_of!(RawProgramHeader, virtual_address))?,
        file_size: read_u64(data, offset_of!(RawProgramHeader, file_size))?,
        memory_size: read_u64(data, offset_of!(RawProgramHeader, memory_size))?,
    })
}
