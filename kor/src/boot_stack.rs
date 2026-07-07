//! Common boot logic — boot stack definition.
//!
//! Architecture-specific entry points live under `src/arch/<arch>/boot.rs`.
//! BSS is cleared in assembly before any Rust code runs.

use core::arch::global_asm;

// Boot stack — 512 KiB, placed in BSS by the linker.
global_asm!(
    ".section .bss.bstack
     .global bstack
     .global bstack_top
     bstack:
     .fill 0x80000
     .size bstack, . - bstack
     bstack_top:"
);
