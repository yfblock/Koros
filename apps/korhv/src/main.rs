#![no_std]
#![no_main]
#![allow(bad_asm_style)]
#![allow(unsafe_op_in_unsafe_fn)]
//! `korhv` -- a minimal Type-1 hypervisor, composed from the same arch-neutral
//! building blocks as `koros` (`kor`, `kor-frame`, `kor-alloc`).
//!
//! Unlike `koros`, this binary crate does not bring up a filesystem, scheduler,
//! timer or SMP; it runs single-CPU with interrupts disabled, enables the
//! hardware virtualization extension (AMD SVM on x86_64, EL2 on aarch64),
//! creates one VM with a stage-2 (NPT / VTTBR) identity mapping, and runs a
//! vCPU that executes a tiny guest image using a hypercall ABI
//! (`HCALL_PUTCHAR` / `HCALL_EXIT`).

extern crate alloc;

mod arch;

use core::arch::global_asm;

// Boot stack — 512 KiB in BSS (owned by this app; kor::boot_stack is gone).
global_asm!(
    ".section .bss.bstack
     .global bstack
     .global bstack_top
     bstack:
     .fill 0x80000
     .size bstack, . - bstack
     bstack_top:"
);

mod boot;
mod heap;
mod hypervisor;
mod panic;

#[global_allocator]
static HEAP: kor_alloc::LockedSlabHeap = kor_alloc::LockedSlabHeap::empty();

/// Hypercall numbers shared by both architectures (the trap instruction and
/// argument registers differ -- see each `arch::<arch>::hyp`).
pub const HCALL_PUTCHAR: usize = 1;
pub const HCALL_EXIT: usize = 2;

#[unsafe(no_mangle)]
extern "C" fn kernel_main() -> ! {
    boot::initialize();

    kor::println!("");
    kor::println!(
        "=== korhv: Type-1 hypervisor ({} KiB free) ===",
        kor::frame::available_frames() * 4
    );
    kor::println!("cmdline: {:?}", kor::cmdline::raw());

    let hypervisor = crate::arch::hypervisor();
    hypervisor.init();
    hypervisor.run();

    kor::println!("=== korhv: guest halted, spinning ===");
    loop {
        core::hint::spin_loop();
    }
}
