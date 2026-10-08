//! x86_64 Multiboot1 boot -- 32-bit entry to long mode, then call the
//! hypervisor `kernel_main`.  Adapted from `kor-arch` (same proven flow).

use core::arch::global_asm;
use x86_64::registers::control::{Cr0Flags, Cr4Flags};
use x86_64::registers::model_specific::EferFlags;

const MULTIBOOT_MAGIC: u32 = 0x1BADB002;
const MULTIBOOT_FLAGS: usize = (1 << 0) | (1 << 6) | (1 << 16);
const IA32_EFER_MSR: u32 = 0xC0000080;
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

#[unsafe(no_mangle)]
extern "C" fn rust_entry(_magic: usize, mbi: usize) {
    super::mm::set_multiboot_info(mbi);
    // `kernel_main` is provided by the korhv binary crate and never returns.
    crate::kernel_main()
}
