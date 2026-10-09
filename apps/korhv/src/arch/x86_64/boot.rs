//! x86_64 Multiboot boot for this hypervisor.
//!
//! Trampoline composed from kor-arch fragments (uniform names); the
//! mapping policy is kor-arch's default tables — identity low 4 GiB +
//! high-half, which the host uses until the NPT stage-2 takes over.
//! `rust_entry` (kor-arch) stashes the Multiboot info and tail-calls
//! `kernel_main`.

core::arch::global_asm!(
    kor_arch::boot_stack_asm!(0x80000),
    kor_arch::boot_setup_asm!(),
    kor_arch::boot_pt_buffer_asm!(),
    kor_arch::boot_cross_asm!(),
    kor_arch::boot_call_asm!(kernel_main),
    kor_arch::boot_secondary_asm!(),
);
