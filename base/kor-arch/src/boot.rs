//! Boot authoring toolkit — shared, arch-neutral pieces.
//!
//! Each architecture contributes identically-named fragment macros in its
//! own `boot_macros` module (`riscv64::boot_macros`, `aarch64::boot_macros`,
//! `x86_64::boot_macros`); only the active architecture's module is
//! compiled, so the crate root always exposes exactly one
//! `kor_arch::boot_setup_asm!` / `boot_cross_asm!` / … per build, and
//! applications compose the same names on every target.
//!
//! Every fragment expands to a string literal that an application splices
//! into its own `global_asm!`, so the boot sequence (and especially the
//! boot *mapping policy*) is app-owned assembly.  kor-arch contributes only
//! the arch *mechanism* (entry prologue, BSS clear, stack switch, MMU
//! enable + offset crossing, secondary-CPU stub).
//!
//! Uniform composition (the mapping-policy slot differs per arch; x86_64's
//! policy is table *data* carried by [`boot_pt_buffer_asm!`]):
//!
//! ```ignore
//! kor_arch::boot_entry!();
//!
//! core::arch::global_asm!(
//!     kor_arch::boot_stack_asm!(0x80000),       // stack, app policy
//!     kor_arch::boot_pt_buffer_asm!(),          // default-mapping artifacts
//!     kor_arch::boot_setup_asm!(),              // _start: BSS clear, sp
//!     "… app mapping policy (raw asm) …",       // slot contract: see below
//!     kor_arch::boot_cross_asm!(),              // MMU enable + crossing
//!     kor_arch::boot_call_asm!(rust_entry),     // enter Rust
//!     kor_arch::boot_secondary_asm!(),          // secondary-CPU stub
//! );
//! ```
//!
//! Mapping-slot contract (riscv64 / aarch64):
//!
//! * entering the slot: BSS cleared, `sp` on the boot stack, firmware args
//!   preserved in callee-saved registers (`s0`/`s1` on riscv64,
//!   `x19`/`x20`/`x24` on aarch64), MMU off, PC physical;
//! * leaving the slot: those registers and `sp` untouched, and the
//!   phys→virt offset in the arch's result register — `a0` (riscv64) or
//!   `x23` (aarch64); `0` means "stay unmapped".  When the offset is
//!   non-zero the leaf tables must live at `_boot_pt_buffer`;
//! * after [`boot_cross_asm!`] falls through, the offset register (`a0` /
//!   `x23`) and — on aarch64 — the load delta `x24` are still live, which
//!   is what lets [`boot_call_asm!`] pick the entry's virtual or physical
//!   address uniformly.

/// Boot stack in `.bss.bstack` (size in bytes, app policy).
///
/// Emits the `bstack` / `bstack_top` symbols referenced by every
/// architecture's `boot_setup_asm!`.
#[macro_export]
macro_rules! boot_stack_asm {
    ($size:expr) => {
        concat!(
            ".section .bss.bstack\n",
            ".global bstack\n",
            ".global bstack_top\n",
            "bstack:\n",
            concat!(".fill ", stringify!($size), "\n"),
            ".size bstack, . - bstack\n",
            "bstack_top:\n",
        )
    };
}
