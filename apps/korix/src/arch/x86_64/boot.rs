//! x86_64 Multiboot boot for this application.
//!
//! Trampoline composed from kor-arch fragments (uniform names).  Long mode
//! requires paging before the first instruction, so the mapping policy is
//! table *data*: this app uses kor-arch's default identity-low-4-GiB +
//! high-half tables (`boot_pt_buffer_asm!` — swap it for custom tables to
//! change the policy).  The tables use the `Page2MTable` macro defined by
//! `boot_setup_asm!`, so they compose after it.

core::arch::global_asm!(
    kor_arch::boot_stack_asm!(0x80000),
    kor_arch::boot_setup_asm!(),
    kor_arch::boot_pt_buffer_asm!(),
    kor_arch::boot_cross_asm!(),
    kor_arch::boot_call_asm!(rust_entry),
    kor_arch::boot_secondary_asm!(),
);
