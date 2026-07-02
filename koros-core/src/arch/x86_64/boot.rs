//! x86_64 boot — Multiboot header, 32→64-bit transition (in assembly).

use core::arch::global_asm;
use x86_64::registers::control::{Cr0Flags, Cr4Flags};
use x86_64::registers::model_specific::EferFlags;

/// Multiboot header magic number.
const MULTIBOOT_MAGIC: u32 = 0x1BADB002;

/// Multiboot flags: memory info (bit 0) + mmap (bit 6) + address fields (bit 16).
const MULTIBOOT_FLAGS: usize = (1 << 0) | (1 << 6) | (1 << 16);

/// IA32_EFER MSR address.
const IA32_EFER_MSR: u32 = 0xC0000080;

/// Virtual address offset — VIRT_ADDR_START from the linker BASE_ADDRESS.
/// Multiboot addresses are computed as `symbol - KERNEL_OFFSET` to get
/// the 32-bit physical addresses that QEMU expects.
const KERNEL_OFFSET: usize = 0xffff_8000_0000_0000;

const CR0: u64 = Cr0Flags::PROTECTED_MODE_ENABLE.bits()
    | Cr0Flags::MONITOR_COPROCESSOR.bits()
    | Cr0Flags::NUMERIC_ERROR.bits()
    | Cr0Flags::WRITE_PROTECT.bits()
    | Cr0Flags::PAGING.bits();

const CR4: u64 = Cr4Flags::PHYSICAL_ADDRESS_EXTENSION.bits()
    | Cr4Flags::PAGE_GLOBAL.bits()
    | Cr4Flags::OSFXSR.bits()
    | Cr4Flags::PAGE_SIZE_EXTENSION.bits()
    | Cr4Flags::OSXMMEXCPT_ENABLE.bits();

const EFER: u64 = EferFlags::LONG_MODE_ENABLE.bits();

global_asm!(
    include_str!("multiboot.S"),
    mb_hdr_magic  = const MULTIBOOT_MAGIC,
    mb_hdr_flags  = const MULTIBOOT_FLAGS,
    entry         = sym rust_entry,
    kernel_offset = const KERNEL_OFFSET,
    cr0           = const CR0,
    cr4           = const CR4,
    efer_msr      = const IA32_EFER_MSR,
    efer          = const EFER,
);

/// Rust entry called from the 64-bit assembly stub.
#[unsafe(no_mangle)]
extern "C" fn rust_entry(_magic: usize, mbi: usize) {
    crate::arch::x86_64::mm::set_multiboot_info(mbi);
    // SAFETY: `kernel_main` is provided by the `koros` binary crate and never
    // returns.
    unsafe { crate::kernel_main() }
}
