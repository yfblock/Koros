//! loongarch64 SMP: CPU identity via the `CPUID` CSR, secondary bring-up via
//! the inter-processor mailbox + IPI (matching the `loongArch64` crate / Linux
//! `csr_mail_send` + `send_ipi_single`, as used by polyhal).
//!
//! For each secondary core we write the entry address into mailbox 0 and the
//! per-CPU stack top into mailbox 1, then send an IPI; the core wakes and jumps
//! to `_secondary_start` (see `boot.rs`), which loads its stack from mailbox 1.

use kor::smp::MAX_CPUS;

/// Per-secondary-CPU kernel stack size.
const SECONDARY_STACK_SIZE: usize = 0x1_0000; // 64 KiB

#[repr(align(16))]
struct Stack([u8; SECONDARY_STACK_SIZE]);

static mut SECONDARY_STACKS: [Stack; MAX_CPUS] =
    [const { Stack([0; SECONDARY_STACK_SIZE]) }; MAX_CPUS];

// IOCSR registers / encodings for the mailbox + IPI mechanism (see the
// `loongArch64` crate `consts.rs`).
const IOCSR_IPI_SEND: usize = 0x1040;
const IOCSR_MBUF_SEND: usize = 0x1048;
const MBUF_SEND_BLOCKING: u64 = 1 << 31;
const MBUF_SEND_BOX_SHIFT: usize = 2;
const MBUF_SEND_CPU_SHIFT: usize = 16;
const MBUF_SEND_BUF_SHIFT: usize = 32;
const MBUF_SEND_H32_MASK: u64 = 0xFFFF_FFFF_0000_0000;
const IPI_SEND_BLOCKING: u32 = 1 << 31;
const IPI_SEND_CPU_SHIFT: u32 = 16;

unsafe extern "C" {
    fn _secondary_start();
}

/// Core id from the `CPUID` CSR (0x20).
pub fn cpu_id() -> usize {
    let id: usize;
    // SAFETY: reads a read-only CSR.
    unsafe { core::arch::asm!("csrrd {}, 0x20", out(reg) id) };
    id & 0x1ff
}

/// No-op: the CPU id is read from hardware.
pub fn set_cpu_id(_id: usize) {}

/// Idle until an interrupt.
pub fn wait_for_interrupt() {
    // SAFETY: `idle` waits for an interrupt.
    unsafe { core::arch::asm!("idle 0") };
}

fn iocsr_write_d(reg: usize, val: u64) {
    // SAFETY: privileged IOCSR access, valid in kernel (PLV0).
    unsafe { core::arch::asm!("iocsrwr.d {v}, {r}", v = in(reg) val, r = in(reg) reg) };
}

fn iocsr_write_w(reg: usize, val: u32) {
    // SAFETY: privileged IOCSR access, valid in kernel (PLV0).
    unsafe { core::arch::asm!("iocsrwr.w {v}, {r}", v = in(reg) val as usize, r = in(reg) reg) };
}

/// Send a 64-bit value to `cpu`'s mailbox `mbox`, high 32 bits then low 32.
fn mail_send(entry: u64, cpu: usize, mbox: usize) {
    let hi = MBUF_SEND_BLOCKING
        | (((mbox << 1) + 1) << MBUF_SEND_BOX_SHIFT) as u64
        | (cpu << MBUF_SEND_CPU_SHIFT) as u64
        | (entry & MBUF_SEND_H32_MASK);
    iocsr_write_d(IOCSR_MBUF_SEND, hi);

    let lo = MBUF_SEND_BLOCKING
        | ((mbox << 1) << MBUF_SEND_BOX_SHIFT) as u64
        | (cpu << MBUF_SEND_CPU_SHIFT) as u64
        | (entry << MBUF_SEND_BUF_SHIFT);
    iocsr_write_d(IOCSR_MBUF_SEND, lo);
}

/// Send IPI action 1 to `cpu`.
fn ipi_send(cpu: usize) {
    let val = IPI_SEND_BLOCKING | ((cpu as u32) << IPI_SEND_CPU_SHIFT) | 1;
    iocsr_write_w(IOCSR_IPI_SEND, val);
}

/// Release every secondary core listed in the device tree.  Returns the number
/// of cores told to start.
pub fn start_secondaries() -> usize {
    let boot = cpu_id();
    let entry = _secondary_start as usize as u64;
    let dtb = kor::config::config_dtb();
    let ncpu = if dtb != 0 {
        // SAFETY: `dtb` is the platform DTB address (direct-mapped).
        unsafe { kor::fdt::cpu_count(kor::arch::phys_to_virt(dtb)) }
    } else {
        1
    }
    .min(MAX_CPUS);

    let mut started = 0;
    for cpu in 0..ncpu {
        if cpu == boot {
            continue;
        }
        let sp = unsafe {
            core::ptr::addr_of!(SECONDARY_STACKS[cpu]) as usize + SECONDARY_STACK_SIZE
        } as u64;
        mail_send(entry, cpu, 0);
        mail_send(sp, cpu, 1);
        ipi_send(cpu);
        started += 1;
    }
    started
}
