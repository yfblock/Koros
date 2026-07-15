//! Architecture-specific virtual address layout for Korix user processes.

pub const PAGE_SIZE: usize = 4096;
pub const LOAD_BASE: usize = 0x0001_0000;
pub const STACK_SIZE: usize = 64 * 1024;

#[cfg(target_arch = "riscv64")]
pub const ADDRESS_LIMIT: usize = 1 << 38;
#[cfg(target_arch = "aarch64")]
pub const ADDRESS_LIMIT: usize = 1 << 48;
#[cfg(target_arch = "x86_64")]
pub const ADDRESS_LIMIT: usize = 1 << 47;

#[cfg(target_arch = "riscv64")]
pub const STACK_TOP: usize = 0x10_0000_0000;
#[cfg(target_arch = "aarch64")]
pub const STACK_TOP: usize = 0x80_0000_0000;
#[cfg(target_arch = "x86_64")]
pub const STACK_TOP: usize = 0x8_0000_0000;

#[cfg(target_arch = "riscv64")]
pub const MMAP_BASE: usize = 0x20_0000_0000;
#[cfg(target_arch = "aarch64")]
pub const MMAP_BASE: usize = 0x80_0000_0000;
#[cfg(target_arch = "x86_64")]
pub const MMAP_BASE: usize = 0x8_0000_0000;

#[cfg(target_arch = "riscv64")]
pub const BRK_BASE: usize = 0x1_0001_0000;
#[cfg(target_arch = "aarch64")]
pub const BRK_BASE: usize = 0x80_0100_0000;
#[cfg(target_arch = "x86_64")]
pub const BRK_BASE: usize = 0x4_0000_1000;

#[cfg(target_arch = "riscv64")]
pub const SIGNAL_RESTORER: usize = 0x0f_fffe_f000;
#[cfg(target_arch = "aarch64")]
pub const SIGNAL_RESTORER: usize = 0x7f_fffe_f000;

const _: () = assert!(LOAD_BASE < ADDRESS_LIMIT);
const _: () = assert!(BRK_BASE < ADDRESS_LIMIT);
const _: () = assert!(MMAP_BASE < ADDRESS_LIMIT);
const _: () = assert!(STACK_TOP <= ADDRESS_LIMIT);
const _: () = assert!(STACK_TOP >= STACK_SIZE);
