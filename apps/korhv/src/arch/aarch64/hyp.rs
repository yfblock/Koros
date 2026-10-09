//! aarch64 EL2 hypervisor -- 2 Linux guests with per-guest stage-2 isolation.
//!
//! Each guest has its own stage-2 page table mapping only its own 64MB RAM + DTB.
//! VTTBR switch + tlbi vmalls12e1is on guest switch ensures physical isolation.
//! Stage-2 traps GICD/GICC/UART MMIO. Emulates GICv2 (256 IRQs) + virtual PL011.
//! CNTV PPI 27 -> vIRQ via HCR_EL2.VI. PSCI via HVC (QEMU intercepts SMC).
//! CNTP kick (PPI 30) breaks tight loops + time-slices (1s each).
//! Stage-1 MMU + CNTV saved/restored on switch (deferred, not per-exit).
//! Key fixes: HPFAR_EL2 << 8 (QEMU TCG IPA bug), tlbi alle2 (stage-2 coherency).

use super::console_mux::{ConsoleMux, InputAction};
use core::arch::{asm, global_asm};

const EC_WFI: u64 = 0x01;
const EC_HVC: u64 = 0x16;
const EC_SMC: u64 = 0x17;
const HCR_EL2_BASE: u64 = (1 << 0) | (1 << 3) | (1 << 31) | (1 << 21); // VM | IMO | RW | TWI
const HCR_TWI: u64 = 1 << 21;
const EC_IABT: u64 = 0x20;
const EC_DABT: u64 = 0x24;
const HCR_VI: u64 = 1 << 7;
const VTCR_EL2_VAL: u64 = 0x19 | (1 << 6) | (1 << 8) | (1 << 10) | (3 << 12) | (2 << 16);
const S2_BLOCK: u64 = 0x1 | (0xF << 2) | (0x3 << 6) | (1 << 10);
const SPSR_EL1H: u64 = 0x3C5;
const GICD_BASE: u64 = 0x0800_0000;
const GICC_BASE: u64 = 0x0801_0000;
const UART_BASE: u64 = 0x0900_0000;
const VIRQ_TIMER: u32 = 27;
const PPI_CNTP: u32 = 30;
const SLICE_TICKS: u64 = 0x80000;
const GUEST_IPA_BASE: u64 = 0x4020_0000;
const GUEST_MEMORY_SIZE: u64 = 0x0400_0000;
const GUEST0_HPA_BASE: u64 = 0x4020_0000;
const GUEST1_HPA_BASE: u64 = 0x4420_0000;
const GUEST_DTB_IPA: u64 = 0x4900_0000;
const GUEST0_DTB_HPA: u64 = 0x4820_0000;
const GUEST1_DTB_HPA: u64 = 0x4840_0000;
const STAGE2_BLOCK_SIZE: u64 = 0x20_0000;

#[derive(Clone, Copy)]
struct GuestManifest {
    ipa_base: u64,
    hpa_base: u64,
    memory_size: u64,
    dtb_ipa: u64,
    dtb_hpa: u64,
}

const GUEST_MANIFESTS: [GuestManifest; 2] = [
    GuestManifest {
        ipa_base: GUEST_IPA_BASE,
        hpa_base: GUEST0_HPA_BASE,
        memory_size: GUEST_MEMORY_SIZE,
        dtb_ipa: GUEST_DTB_IPA,
        dtb_hpa: GUEST0_DTB_HPA,
    },
    GuestManifest {
        ipa_base: GUEST_IPA_BASE,
        hpa_base: GUEST1_HPA_BASE,
        memory_size: GUEST_MEMORY_SIZE,
        dtb_ipa: GUEST_DTB_IPA,
        dtb_hpa: GUEST1_DTB_HPA,
    },
];

struct GuestMemory(GuestManifest);

impl GuestMemory {
    fn translate(&self, gpa: u64, length: usize) -> Option<u64> {
        let length = u64::try_from(length).ok()?;
        let end = gpa.checked_add(length)?;
        let memory_end = self.0.ipa_base.checked_add(self.0.memory_size)?;
        if gpa < self.0.ipa_base || end > memory_end {
            return None;
        }
        self.0.hpa_base.checked_add(gpa - self.0.ipa_base)
    }
}

struct Stage2Root {
    l1: usize,
    l2_0: usize,
    l2_1: usize,
}

impl Stage2Root {
    fn root(&self) -> usize {
        self.l1
    }
}

impl Drop for Stage2Root {
    fn drop(&mut self) {
        kor::frame::free_frames(self.l2_1, 1);
        kor::frame::free_frames(self.l2_0, 1);
        kor::frame::free_frames(self.l1, 1);
    }
}

const PSCI_VERSION: u32 = 0x8400_0000;
const PSCI_CPU_ON: u32 = 0xC400_0003;
const PSCI_MIGRATE_INFO_TYPE: u32 = 0x8400_0006;
const PSCI_SYSTEM_OFF: u32 = 0x8400_0008;
const PSCI_SYSTEM_RESET: u32 = 0xC400_0009;
const PSCI_FEATURES: u32 = 0x8400_000A;
const PSCI_NOT_SUPPORTED: u64 = !0;

extern crate alloc;

#[repr(C)]
struct Vcpu {
    regs: [u64; 31],
    sp: u64,
    elr: u64,
    spsr: u64,
    esr: u64,
    host: [u64; 13],
    fp: [u128; 32],
    fpcr: u64,
    fpsr: u64,
}

const _: () = assert!(core::mem::offset_of!(Vcpu, fp) == 384);
const _: () = assert!(core::mem::offset_of!(Vcpu, fpcr) == 896);
const _: () = assert!(core::mem::offset_of!(Vcpu, fpsr) == 904);

struct Vgic {
    gicd_ctlr: u32,
    enabled: [u32; 8],
    pending: [u32; 8],
    prio: [u8; 256],
    gicc_ctlr: u32,
    gicc_pmr: u32,
    active: u32,
}
impl Vgic {
    const fn new() -> Self {
        Self {
            gicd_ctlr: 0,
            enabled: [0; 8],
            pending: [0; 8],
            prio: [0; 256],
            gicc_ctlr: 0,
            gicc_pmr: 0,
            active: 0x3FF,
        }
    }
    fn inject(&mut self, irq: u32) {
        self.pending[(irq / 32) as usize] |= 1 << (irq % 32);
    }
    fn has_pending(&self) -> bool {
        (0..8).any(|i| self.pending[i] & self.enabled[i] != 0)
    }
    fn iar(&mut self) -> u32 {
        for i in 0..8 {
            let m = self.pending[i] & self.enabled[i];
            if m != 0 {
                let irq = i as u32 * 32 + m.trailing_zeros();
                self.pending[i] &= !(1 << (irq % 32));
                self.active = irq;
                return irq;
            }
        }
        0x3FF
    }
    fn eoir(&mut self, irq: u32) {
        if self.active == irq {
            self.active = 0x3FF;
        }
    }
}

struct Vuart {
    cr: u32,
    lcr_h: u32,
    imsc: u32,
    ibrd: u32,
    ifls: u32,
    txris: bool,
    rxbuf: [u8; 256],
    rxlen: usize,
    rxhead: usize,
}
impl Vuart {
    const fn new() -> Self {
        Self {
            cr: 0,
            lcr_h: 0,
            imsc: 0,
            ibrd: 0,
            ifls: 0,
            txris: false,
            rxbuf: [0; 256],
            rxlen: 0,
            rxhead: 0,
        }
    }
    fn rx_push(&mut self, ch: u8) {
        if self.rxlen < 256 {
            let tail = (self.rxhead + self.rxlen) & 0xFF;
            self.rxbuf[tail] = ch;
            self.rxlen += 1;
        }
    }
    fn rx_pop(&mut self) -> Option<u8> {
        if self.rxlen > 0 {
            let ch = self.rxbuf[self.rxhead];
            self.rxhead = (self.rxhead + 1) & 0xFF;
            self.rxlen -= 1;
            Some(ch)
        } else {
            None
        }
    }
    fn has_rx(&self) -> bool {
        self.rxlen > 0
    }
}

global_asm!(".arch armv8-a+fp");
global_asm!(
    ".align 11",
    ".global el2_vectors",
    "el2_vectors:",
    "    b host_fault",
    ".align 7",
    "    b host_fault",
    ".align 7",
    "    b host_fault",
    ".align 7",
    "    b host_fault",
    ".align 7",
    "    b host_fault",
    ".align 7",
    "    b host_fault",
    ".align 7",
    "    b host_fault",
    ".align 7",
    "    b host_fault",
    ".align 7",
    "    b el2_guest_exit",
    ".align 7",
    "    b el2_guest_irq_exit",
    ".align 7",
    "    b el2_guest_exit",
    ".align 7",
    "    b el2_guest_exit",
    ".align 7",
    "    b host_fault",
    ".align 7",
    "    b host_fault",
    ".align 7",
    "    b host_fault",
    ".align 7",
    "    b host_fault",
    ".align 7",
    ".global vcpu_enter",
    "vcpu_enter:",
    "    stp x19, x20, [x0, #280]",
    "    stp x21, x22, [x0, #296]",
    "    stp x23, x24, [x0, #312]",
    "    stp x25, x26, [x0, #328]",
    "    stp x27, x28, [x0, #344]",
    "    stp x29, x30, [x0, #360]",
    "    mov x9, sp",
    "    str x9, [x0, #376]",
    "    msr tpidr_el2, x0",
    "    ldr x9, [x0, #248]",
    "    msr sp_el1, x9",
    "    ldr x10, [x0, #256]",
    "    msr elr_el2, x10",
    "    ldr x11, [x0, #264]",
    "    msr spsr_el2, x11",
    "    ldp q0, q1, [x0, #384]",
    "    ldp q2, q3, [x0, #416]",
    "    ldp q4, q5, [x0, #448]",
    "    ldp q6, q7, [x0, #480]",
    "    ldp q8, q9, [x0, #512]",
    "    ldp q10, q11, [x0, #544]",
    "    ldp q12, q13, [x0, #576]",
    "    ldp q14, q15, [x0, #608]",
    "    ldp q16, q17, [x0, #640]",
    "    ldp q18, q19, [x0, #672]",
    "    ldp q20, q21, [x0, #704]",
    "    ldp q22, q23, [x0, #736]",
    "    ldp q24, q25, [x0, #768]",
    "    ldp q26, q27, [x0, #800]",
    "    ldp q28, q29, [x0, #832]",
    "    ldp q30, q31, [x0, #864]",
    "    ldr x9, [x0, #896]",
    "    msr fpcr, x9",
    "    ldr x9, [x0, #904]",
    "    msr fpsr, x9",
    "    ldp x1, x2, [x0, #8]",
    "    ldp x3, x4, [x0, #24]",
    "    ldp x5, x6, [x0, #40]",
    "    ldp x7, x8, [x0, #56]",
    "    ldp x12, x13, [x0, #96]",
    "    ldp x14, x15, [x0, #112]",
    "    ldp x16, x17, [x0, #128]",
    "    ldr x18, [x0, #144]",
    "    ldp x19, x20, [x0, #152]",
    "    ldp x21, x22, [x0, #168]",
    "    ldp x23, x24, [x0, #184]",
    "    ldp x25, x26, [x0, #200]",
    "    ldp x27, x28, [x0, #216]",
    "    ldp x29, x30, [x0, #232]",
    "    ldp x9, x10, [x0, #72]",
    "    ldr x11, [x0, #88]",
    "    ldr x0, [x0, #0]",
    "    msr tpidrro_el0, xzr",
    "    eret",
    ".global el2_guest_exit",
    "el2_guest_exit:",
    "    msr tpidrro_el0, xzr",
    "    b el2_guest_exit_common",
    ".global el2_guest_irq_exit",
    "el2_guest_irq_exit:",
    "    str x16, [sp, #-16]!",
    "    mov x16, #1",
    "    msr tpidrro_el0, x16",
    "    ldr x16, [sp], #16",
    "el2_guest_exit_common:",
    "    str x18, [sp, #-16]!",
    "    mrs x18, tpidr_el2",
    "    stp x0, x1, [x18, #0]",
    "    stp x2, x3, [x18, #16]",
    "    stp x4, x5, [x18, #32]",
    "    stp x6, x7, [x18, #48]",
    "    stp x8, x9, [x18, #64]",
    "    stp x10, x11, [x18, #80]",
    "    stp x12, x13, [x18, #96]",
    "    stp x14, x15, [x18, #112]",
    "    stp x16, x17, [x18, #128]",
    "    ldr x1, [sp], #16",
    "    str x1, [x18, #144]",
    "    stp x19, x20, [x18, #152]",
    "    stp x21, x22, [x18, #168]",
    "    stp x23, x24, [x18, #184]",
    "    stp x25, x26, [x18, #200]",
    "    stp x27, x28, [x18, #216]",
    "    stp x29, x30, [x18, #232]",
    "    mrs x1, sp_el1",
    "    str x1, [x18, #248]",
    "    mrs x1, elr_el2",
    "    str x1, [x18, #256]",
    "    mrs x1, spsr_el2",
    "    str x1, [x18, #264]",
    "    mrs x1, tpidrro_el0",
    "    cbz x1, 1f",
    "    mov x1, #1",
    "    lsl x1, x1, #63",
    "    b 2f",
    "1:  mrs x1, esr_el2",
    "2:  str x1, [x18, #272]",
    "    stp q0, q1, [x18, #384]",
    "    stp q2, q3, [x18, #416]",
    "    stp q4, q5, [x18, #448]",
    "    stp q6, q7, [x18, #480]",
    "    stp q8, q9, [x18, #512]",
    "    stp q10, q11, [x18, #544]",
    "    stp q12, q13, [x18, #576]",
    "    stp q14, q15, [x18, #608]",
    "    stp q16, q17, [x18, #640]",
    "    stp q18, q19, [x18, #672]",
    "    stp q20, q21, [x18, #704]",
    "    stp q22, q23, [x18, #736]",
    "    stp q24, q25, [x18, #768]",
    "    stp q26, q27, [x18, #800]",
    "    stp q28, q29, [x18, #832]",
    "    stp q30, q31, [x18, #864]",
    "    mrs x1, fpcr",
    "    str x1, [x18, #896]",
    "    mrs x1, fpsr",
    "    str x1, [x18, #904]",
    "    ldp x19, x20, [x18, #280]",
    "    ldp x21, x22, [x18, #296]",
    "    ldp x23, x24, [x18, #312]",
    "    ldp x25, x26, [x18, #328]",
    "    ldp x27, x28, [x18, #344]",
    "    ldp x29, x30, [x18, #360]",
    "    ldr x1, [x18, #376]",
    "    mov sp, x1",
    "    ldr x0, [x18, #272]",
    "    ret",
    ".global host_fault",
    "host_fault:",
    "    mov x0, x30",
    "    bl host_fault_handler",
    "1:  wfe",
    "    b 1b",
);

unsafe extern "C" {
    fn el2_vectors();
    fn vcpu_enter(vcpu: *mut Vcpu) -> u64;
}

#[unsafe(no_mangle)]
extern "C" fn host_fault_handler(lr: u64) {
    let esr: u64;
    unsafe { asm!("mrs {}, esr_el2", out(reg) esr) };
    let elr: u64;
    unsafe { asm!("mrs {}, elr_el2", out(reg) elr) };
    let far: u64;
    unsafe { asm!("mrs {}, far_el2", out(reg) far) };
    kor::println!(
        "!!! HOST FAULT lr={:#x} esr={:#x} elr={:#x} far={:#x}",
        lr,
        esr,
        elr,
        far
    );
}

pub fn trap_init() {
    unsafe {
        asm!("msr vbar_el2, {}", in(reg) el2_vectors as *const () as u64);
        asm!("isb");
    }
    kor::println!("trap: VBAR_EL2 set");
}

pub fn init() {
    unsafe {
        asm!("msr hcr_el2, {}", in(reg) HCR_EL2_BASE);
        asm!("msr vtcr_el2, {}", in(reg) VTCR_EL2_VAL);
        asm!("msr sctlr_el1, xzr");
        asm!("msr cpacr_el1, {}", in(reg) 0x300000u64); // FPEN=1 (FPSIMD from EL0/EL1)
        asm!("msr cptr_el2, xzr"); // No trap on FP/SIMD at EL2
        asm!("msr cnthctl_el2, {}", in(reg) 1u64); // EL1PCTEN=1 (counter OK), EL1PCEN=0 (timer trapped)
        let mpidr: u64;
        asm!("mrs {}, mpidr_el1", out(reg) mpidr);
        asm!("msr vmpidr_el2, {}", in(reg) mpidr);
        asm!("msr cntv_ctl_el0, xzr"); // disable guest virtual timer
        // Enable real PL011 UART for RX so QEMU chardev can deliver stdin
        kor::PhysAddr::new((UART_BASE + 0x02c) as usize)
            .to_ka()
            .write_u32(0x70); // UARTLCR_H: 8N1, FIFO on
        kor::PhysAddr::new((UART_BASE + 0x030) as usize)
            .to_ka()
            .write_u32(0x301); // UARTCR: UARTEN|TXE|RXE
    }
    // Real GIC for hypervisor interrupts (PPI 27 timer, PPI 30 CNTP).
    // Distributor: enable both groups, the guest-timer PPI, and all SPIs
    // (routed to CPU 0, Group 1, priority 0).
    kor::PhysAddr::new(GICD_BASE as usize).to_ka().write_u32(3); // GICD_CTLR
    kor::PhysAddr::new(GICD_BASE as usize + 0x100)
        .to_ka()
        .write_u32(1 << VIRQ_TIMER); // ISENABLER0
    for i in 32..64usize {
        kor::PhysAddr::new(GICD_BASE as usize + 0x800 + i)
            .to_ka()
            .write_u8(1); // ITARGETSR
    }
    kor::PhysAddr::new(GICD_BASE as usize + 0x84)
        .to_ka()
        .write_u32(0xFFFF_FFFF); // IGROUPR1
    kor::PhysAddr::new(GICD_BASE as usize + 0x104)
        .to_ka()
        .write_u32(0xFFFF_FFFF); // ISENABLER1
    for i in 32..64usize {
        kor::PhysAddr::new(GICD_BASE as usize + 0x400 + i)
            .to_ka()
            .write_u8(0); // IPRIORITYR
    }
    // CPU interface: enable both groups, mask all priorities.
    kor::PhysAddr::new(GICC_BASE as usize).to_ka().write_u32(3); // GICC_CTLR
    kor::PhysAddr::new(GICC_BASE as usize + 0x4)
        .to_ka()
        .write_u32(0xFF); // GICC_PMR
    kor::println!(
        "el2: Linux stage-2 + vGIC + UART ON (HCR={:#x})",
        HCR_EL2_BASE
    );
}

fn build_stage2(manifest: GuestManifest) -> Stage2Root {
    assert_eq!(manifest.ipa_base % STAGE2_BLOCK_SIZE, 0);
    assert_eq!(manifest.hpa_base % STAGE2_BLOCK_SIZE, 0);
    assert_eq!(manifest.memory_size % STAGE2_BLOCK_SIZE, 0);
    assert_eq!(manifest.dtb_ipa % STAGE2_BLOCK_SIZE, 0);
    assert_eq!(manifest.dtb_hpa % STAGE2_BLOCK_SIZE, 0);
    let l1 = kor::frame::alloc_frame().expect("s2 L1");
    let l2_0 = kor::frame::alloc_frame().expect("s2 L2_0");
    let l2_1 = kor::frame::alloc_frame().expect("s2 L2_1");
    let l1v = crate::arch::mm::phys_to_virt(l1) as *mut u64;
    let l2_0v = crate::arch::mm::phys_to_virt(l2_0) as *mut u64;
    let l2_1v = crate::arch::mm::phys_to_virt(l2_1) as *mut u64;
    unsafe {
        core::ptr::write_bytes(l1v.cast::<u8>(), 0, 4096);
        core::ptr::write_bytes(l2_0v.cast::<u8>(), 0, 4096);
        core::ptr::write_bytes(l2_1v.cast::<u8>(), 0, 4096);
        l1v.add(0).write_volatile((l2_0 as u64) | 0x3);
        l1v.add(1).write_volatile((l2_1 as u64) | 0x3);

        let start = ((manifest.ipa_base - 0x4000_0000) / STAGE2_BLOCK_SIZE) as usize;
        let count = (manifest.memory_size / STAGE2_BLOCK_SIZE) as usize;
        for i in 0..count {
            l2_1v
                .add(start + i)
                .write_volatile(manifest.hpa_base + i as u64 * STAGE2_BLOCK_SIZE | S2_BLOCK);
        }
        // Linux updates boot-only properties in its private DTB during early boot.
        let dtb_index = ((manifest.dtb_ipa - 0x4000_0000) / STAGE2_BLOCK_SIZE) as usize;
        l2_1v
            .add(dtb_index)
            .write_volatile(manifest.dtb_hpa | S2_BLOCK);

        for off in (0..4096).step_by(64) {
            asm!("dc civac, {}", in(reg) l1v as usize + off);
            asm!("dc civac, {}", in(reg) l2_0v as usize + off);
            asm!("dc civac, {}", in(reg) l2_1v as usize + off);
        }
        asm!("dsb sy");
        asm!("tlbi alle2");
        asm!("dsb sy");
        asm!("isb");
    }
    Stage2Root { l1, l2_0, l2_1 }
}

fn gicd_read(v: &Vgic, off: usize) -> u32 {
    match off {
        0x000 => v.gicd_ctlr,
        0x004 => 7,
        0x008 => 0,
        0x080..=0x09c => 0,
        0x100..=0x11c => v.enabled[(off - 0x100) / 4],
        0x180..=0x19c => v.enabled[(off - 0x180) / 4],
        0x200..=0x21c => v.pending[(off - 0x200) / 4],
        0x280..=0x29c => v.pending[(off - 0x280) / 4],
        0x300..=0x31c => 0,
        0x380..=0x39c => 0,
        0x400..=0x4ff => v.prio[off - 0x400] as u32,
        0x800..=0x8ff => 1,
        0xc00..=0xc3c => 0xaaaaaaaa,
        _ => 0,
    }
}
fn gicd_write(v: &mut Vgic, off: usize, val: u32) {
    match off {
        0x000 => v.gicd_ctlr = val,
        0x100..=0x11c => v.enabled[(off - 0x100) / 4] |= val,
        0x180..=0x19c => v.enabled[(off - 0x180) / 4] &= !val,
        0x200..=0x21c => v.pending[(off - 0x200) / 4] |= val,
        0x280..=0x29c => v.pending[(off - 0x280) / 4] &= !val,
        0x400..=0x4ff => v.prio[off - 0x400] = val as u8,
        _ => {}
    }
}
fn gicc_read(v: &mut Vgic, off: usize) -> u32 {
    match off {
        0x000 => v.gicc_ctlr,
        0x004 => v.gicc_pmr,
        0x008 => 0,
        0x00c => v.iar(),
        0x010 => 0,
        0x014 => 0,
        0x018 => {
            let m = (0..8).find_map(|i| {
                let x = v.pending[i] & v.enabled[i];
                if x != 0 {
                    Some(i as u32 * 32 + x.trailing_zeros())
                } else {
                    None
                }
            });
            m.unwrap_or(0x3FF)
        }
        0x01c => 0,
        _ => 0,
    }
}
fn gicc_write(v: &mut Vgic, off: usize, val: u32) {
    match off {
        0x000 => v.gicc_ctlr = val,
        0x004 => v.gicc_pmr = val,
        0x010 => v.eoir(val),
        _ => {}
    }
}
fn uart_read(vu: &mut Vuart, off: usize) -> u32 {
    match off {
        0x000 => vu.rx_pop().map(|c| c as u32).unwrap_or(0), // UARTDR: pop RX char
        0x004 => 0,
        0x018 => {
            // UARTFR: bit4=TXFE, bit6=RXFE; RXFE=0 when we have buffered input
            if vu.has_rx() { 0x10 } else { 0x90 }
        }
        0x024 => 0,
        0x028 => vu.ibrd,
        0x02c => vu.lcr_h,
        0x030 => vu.cr,
        0x034 => vu.ifls,
        0x038 => {
            // UARTRIS: bit0=RXRIS, bit1=TXRIS
            (if vu.has_rx() { 0x1 } else { 0 }) | (if vu.txris { 0x2 } else { 0 })
        }
        0x03c => {
            // UARTMIS = RIS & IMSC
            let ris = (if vu.has_rx() { 0x1 } else { 0 }) | (if vu.txris { 0x2 } else { 0 });
            ris & vu.imsc
        }
        0x040 => 0,
        0x044 => vu.imsc,
        0x048 => 0,
        0xfe0 => 0x11,
        0xfe4 => 0x10,
        0xfe8 => 0x14,
        0xfec => 0x00,
        0xff0 => 0x0d,
        0xff4 => 0xf0,
        0xff8 => 0x05,
        0xffc => 0xb1,
        _ => 0,
    }
}

fn uart_write(vu: &mut Vuart, off: usize, val: u32, vgic: &mut Vgic) {
    match off {
        0x000 => {
            kor::PhysAddr::new(UART_BASE as usize)
                .to_ka()
                .write_u8(val as u8);
        }
        0x004 => {}
        0x024 => {}
        0x028 => vu.ibrd = val,
        0x02c => vu.lcr_h = val,
        0x030 => {
            vu.cr = val;
        } // Store in vUART only — real PL011 stays RX-enabled
        0x034 => vu.ifls = val,
        0x040 => {
            if val & 0x2 != 0 {
                vu.txris = false;
            }
        } // UARTICR: clear TXRIS
        0x044 => {
            vu.imsc = val | 0x1; // Force RXIM on — PL011 driver writes 0x7d0 without it
            if val & 0x2 != 0 {
                vu.txris = true;
                vgic.inject(33);
            }
            if vu.has_rx() {
                vgic.inject(33);
            }
        }
        0x048 => {}
        _ => {}
    }
}

fn handle_mmio(
    vcpu: &mut Vcpu,
    esr: u64,
    vgic: &mut Vgic,
    vuart: &mut Vuart,
    gid: usize,
    memory: &GuestMemory,
) -> bool {
    let iss = esr & 0x1FFFFFF;
    let isv = (iss >> 24) & 1;
    let srt = ((iss >> 16) & 0x1F) as usize;
    let sf = (iss >> 15) & 1;
    let wnr = (iss >> 6) & 1;
    let far: u64;
    let hpfar: u64;
    unsafe {
        asm!("mrs {}, far_el2", out(reg) far);
        asm!("mrs {}, hpfar_el2", out(reg) hpfar);
    }
    let ipa = (hpfar << 8) | (far & 0xFFF);
    let val_in = if srt < 31 {
        if sf == 1 {
            vcpu.regs[srt]
        } else {
            vcpu.regs[srt] & 0xFFFFFFFF
        }
    } else {
        0
    };
    if isv == 0 {
        return false;
    }
    if ipa >= GICD_BASE && ipa < GICD_BASE + 0x10000 {
        let off = (ipa - GICD_BASE) as usize;
        if wnr == 1 {
            gicd_write(vgic, off, val_in as u32);
        } else {
            let r = gicd_read(vgic, off) as u64;
            if srt < 31 {
                vcpu.regs[srt] = if sf == 1 { r } else { r & 0xFFFFFFFF };
            }
        }
    } else if ipa >= GICC_BASE && ipa < GICC_BASE + 0x10000 {
        let off = (ipa - GICC_BASE) as usize;
        if wnr == 1 {
            gicc_write(vgic, off, val_in as u32);
        } else {
            let r = gicc_read(vgic, off) as u64;
            if srt < 31 {
                vcpu.regs[srt] = if sf == 1 { r } else { r & 0xFFFFFFFF };
            }
        }
    } else if ipa >= UART_BASE && ipa < UART_BASE + 0x1000 {
        let off = (ipa - UART_BASE) as usize;
        if wnr == 1 {
            uart_write(vuart, off, val_in as u32, vgic);
        } else {
            let r = uart_read(vuart, off) as u64;
            if srt < 31 {
                vcpu.regs[srt] = if sf == 1 { r } else { r & 0xFFFFFFFF };
            }
        }
    } else if ipa >= 0x0a00_0000 && ipa < 0x0a00_0200 {
        // virtio-mmio: forward to per-guest physical device (G0: 0x0a000000, G1: 0x0a000200)
        let dev_base = 0x0a00_0000u64 + (gid as u64) * 0x200;
        let off = (ipa - 0x0a00_0000) as usize;
        let spi_bit = 1u32 << (16 + gid); // SPI 48 for G0, SPI 49 for G1
        if wnr == 1 {
            if off == 0x050 {
                // Validate and temporarily translate every descriptor before
                // exposing it to the physical bus-mastering device.
                let qpn = kor::PhysAddr::new((dev_base + 0x040) as usize)
                    .to_ka()
                    .read_u32() as u64;
                let queue_size = kor::PhysAddr::new((dev_base + 0x038) as usize)
                    .to_ka()
                    .read_u32() as usize;
                if queue_size == 0 || queue_size > 1024 {
                    return false;
                }
                let desc_gpa = qpn.checked_shl(12).unwrap_or(u64::MAX);
                let desc_bytes = queue_size.checked_mul(16).unwrap_or(usize::MAX);
                let Some(desc_hpa) = memory.translate(desc_gpa, desc_bytes) else {
                    return false;
                };
                let desc_va = crate::arch::mm::phys_to_virt(desc_hpa as usize) as *mut u8;
                let mut saved: alloc::vec::Vec<(u64, u64)> = alloc::vec::Vec::new();
                for i in 0..queue_size {
                    let descriptor = unsafe { desc_va.add(i * 16) };
                    let address_ptr = descriptor.cast::<u64>();
                    let address = unsafe { address_ptr.read_volatile() };
                    let length =
                        unsafe { (descriptor.add(8) as *const u32).read_volatile() } as usize;
                    let flags = unsafe { (descriptor.add(12) as *const u16).read_volatile() };
                    let Some(host_address) = memory.translate(address, length) else {
                        return false;
                    };
                    saved.push((address_ptr as u64, address));
                    unsafe {
                        address_ptr.write_volatile(host_address);
                    }
                    if flags & 4 != 0 {
                        if length == 0 || length % 16 != 0 || length / 16 > queue_size {
                            return false;
                        }
                        let indirect_va =
                            crate::arch::mm::phys_to_virt(host_address as usize) as *mut u8;
                        for j in 0..length / 16 {
                            let indirect = unsafe { indirect_va.add(j * 16) };
                            let indirect_ptr = indirect.cast::<u64>();
                            let indirect_address = unsafe { indirect_ptr.read_volatile() };
                            let indirect_length =
                                unsafe { (indirect.add(8) as *const u32).read_volatile() } as usize;
                            let Some(indirect_hpa) =
                                memory.translate(indirect_address, indirect_length)
                            else {
                                return false;
                            };
                            saved.push((indirect_ptr as u64, indirect_address));
                            unsafe {
                                indirect_ptr.write_volatile(indirect_hpa);
                            }
                        }
                    }
                }
                unsafe {
                    asm!("dsb sy");
                }
                kor::PhysAddr::new((dev_base + off as u64) as usize)
                    .to_ka()
                    .write_u32(val_in as u32);
                for _ in 0..200000 {
                    let isp1 = unsafe { ((GICD_BASE + 0x204) as *const u32).read_volatile() };
                    if isp1 & spi_bit != 0 {
                        vgic.inject(48);
                        break;
                    }
                }
                for &(pointer, original) in saved.iter().rev() {
                    unsafe {
                        (pointer as *mut u64).write_volatile(original);
                    }
                }
            } else {
                let value = if off == 0x040 {
                    let queue_gpa = val_in.checked_shl(12).unwrap_or(u64::MAX);
                    let Some(queue_hpa) = memory.translate(queue_gpa, 4096) else {
                        return false;
                    };
                    queue_hpa >> 12
                } else {
                    val_in
                };
                kor::PhysAddr::new((dev_base + off as u64) as usize)
                    .to_ka()
                    .write_u32(value as u32);
                if off == 0x050 {
                    for _ in 0..200000 {
                        let isp1 = unsafe { ((GICD_BASE + 0x204) as *const u32).read_volatile() };
                        if isp1 & spi_bit != 0 {
                            vgic.inject(48);
                            break;
                        }
                    }
                }
            }
        } else {
            let r = unsafe { ((dev_base + off as u64) as *const u32).read_volatile() } as u64;
            if srt < 31 {
                vcpu.regs[srt] = if sf == 1 { r } else { r & 0xFFFFFFFF };
            }
        }
    } else if ipa == 0x0a01_0000 {
        // Magic getchar MMIO: read returns next char from real UART (or 0xffffffff)
        // Accessible from user space via mmap(/dev/mem) — no HVC needed
        if wnr == 0 {
            let fr = unsafe { ((UART_BASE + 0x18) as *const u32).read_volatile() };
            if fr & 0x10 == 0 {
                let ch = unsafe { (UART_BASE as *const u8).read_volatile() } as u64;

                if srt < 31 {
                    vcpu.regs[srt] = if ch != 0 { ch } else { 0xffffffff };
                }
            } else {
                let ch = vuart.rx_pop();
                if srt < 31 {
                    vcpu.regs[srt] = match ch {
                        Some(c) if c != 0 => c as u64,
                        _ => 0xffffffff,
                    };
                }
            }
        }
    } else {
        if ipa >= 0x4000_0000 && ipa < 0x8000_0000 {
            return false;
        }
    }
    vcpu.elr += 4;
    true
}
fn handle_psci(vcpu: &mut Vcpu) -> bool {
    let func = vcpu.regs[0] as u32;
    match func {
        PSCI_VERSION => vcpu.regs[0] = 0x0001_0001,
        PSCI_MIGRATE_INFO_TYPE => vcpu.regs[0] = 2,
        PSCI_FEATURES => {
            let f = vcpu.regs[1] as u32;
            vcpu.regs[0] = match f {
                PSCI_VERSION
                | PSCI_MIGRATE_INFO_TYPE
                | PSCI_FEATURES
                | PSCI_SYSTEM_OFF
                | PSCI_SYSTEM_RESET => 0,
                _ => PSCI_NOT_SUPPORTED,
            };
        }
        PSCI_CPU_ON => vcpu.regs[0] = PSCI_NOT_SUPPORTED,
        PSCI_SYSTEM_OFF => {
            kor::println!("\nel2: Linux PSCI SYSTEM_OFF");
            return true;
        }
        PSCI_SYSTEM_RESET => {
            kor::println!("\nel2: Linux PSCI SYSTEM_RESET");
            return true;
        }
        _ => vcpu.regs[0] = PSCI_NOT_SUPPORTED,
    }
    false
}

struct Guest {
    vcpu: Vcpu,
    memory: GuestMemory,
    stage2: Stage2Root,
    vgic: Vgic,
    vuart: Vuart,
    cntv_ctl: u64,
    cntv_cval: u64,
    done: bool,
    sctlr_el1: u64,
    ttbr0_el1: u64,
    ttbr1_el1: u64,
    tcr_el1: u64,
    mair_el1: u64,
    tpidr_el0: u64,
    tpidr_el1: u64,
    sp_el0: u64,
    vbar_el1: u64,
    cpacr_el1: u64,
    started: bool,
}

impl Guest {
    fn new(manifest: GuestManifest) -> Self {
        let stage2 = build_stage2(manifest);
        let memory = GuestMemory(manifest);
        let mut vcpu = Vcpu {
            regs: [0; 31],
            sp: 0,
            elr: manifest.ipa_base,
            spsr: SPSR_EL1H,
            esr: 0,
            host: [0; 13],
            fp: [0; 32],
            fpcr: 0,
            fpsr: 0,
        };
        vcpu.regs[0] = manifest.dtb_ipa;
        Self {
            vcpu,
            memory,
            stage2,
            vgic: Vgic::new(),
            vuart: Vuart::new(),
            cntv_ctl: 0,
            cntv_cval: 0,
            done: false,
            sctlr_el1: 0,
            ttbr0_el1: 0,
            ttbr1_el1: 0,
            tcr_el1: 0,
            mair_el1: 0,
            tpidr_el0: 0,
            tpidr_el1: 0,
            sp_el0: 0,
            vbar_el1: 0,
            cpacr_el1: 0x300000,
            started: false,
        }
    }
}
pub fn run() {
    let mut gs = [
        Guest::new(GUEST_MANIFESTS[0]),
        Guest::new(GUEST_MANIFESTS[1]),
    ];
    let mut console = ConsoleMux::new();
    console.print_banner();
    let mut cur: usize = 0;
    let mut prev_cur: usize = 2;
    kor::println!("el2: 2 Linux guests with owned stage-2 roots and private RAM/DTB");
    assert!(gs[0].memory.0.hpa_base != gs[1].memory.0.hpa_base);
    assert!(gs[0].memory.0.dtb_hpa != gs[1].memory.0.dtb_hpa);
    assert!(gs[0].stage2.root() != gs[1].stage2.root());
    // Set initial VTTBR to G0's stage-2
    unsafe {
        asm!("tlbi vmalls12e1is");
        asm!("dsb sy");
        asm!("isb");
    }
    loop {
        if gs[0].done && gs[1].done {
            break;
        }
        if gs[cur].done {
            cur = 1 - cur;
            continue;
        }
        unsafe {
            if cur != prev_cur {
                if prev_cur < 2 {
                    asm!("mrs {}, sctlr_el1", out(reg) gs[prev_cur].sctlr_el1);
                    asm!("mrs {}, ttbr0_el1", out(reg) gs[prev_cur].ttbr0_el1);
                    asm!("mrs {}, ttbr1_el1", out(reg) gs[prev_cur].ttbr1_el1);
                    asm!("mrs {}, tcr_el1", out(reg) gs[prev_cur].tcr_el1);
                    asm!("mrs {}, mair_el1", out(reg) gs[prev_cur].mair_el1);
                    asm!("mrs {}, tpidr_el0", out(reg) gs[prev_cur].tpidr_el0);
                    asm!("mrs {}, tpidr_el1", out(reg) gs[prev_cur].tpidr_el1);
                    asm!("mrs {}, vbar_el1", out(reg) gs[prev_cur].vbar_el1);
                    asm!("mrs {}, cpacr_el1", out(reg) gs[prev_cur].cpacr_el1);
                    asm!("mrs {}, sp_el0", out(reg) gs[prev_cur].sp_el0);
                    asm!("mrs {}, cntv_ctl_el0", out(reg) gs[prev_cur].cntv_ctl);
                    asm!("mrs {}, cntv_cval_el0", out(reg) gs[prev_cur].cntv_cval);
                }
                asm!("msr vttbr_el2, {}", in(reg) gs[cur].stage2.root() as u64 | ((cur as u64) << 48));
                let first_run = !gs[cur].started;
                if first_run {
                    asm!("tlbi vmalls12e1is");
                    asm!("dsb sy");
                    asm!("isb");
                }
                asm!("msr ttbr0_el1, {}", in(reg) gs[cur].ttbr0_el1);
                asm!("msr ttbr1_el1, {}", in(reg) gs[cur].ttbr1_el1);
                {
                    let v: u64;
                    asm!("mrs {}, tcr_el1", out(reg) v);
                    if v != gs[cur].tcr_el1 {
                        asm!("msr tcr_el1, {}", in(reg) gs[cur].tcr_el1);
                    }
                }
                {
                    let v: u64;
                    asm!("mrs {}, mair_el1", out(reg) v);
                    if v != gs[cur].mair_el1 {
                        asm!("msr mair_el1, {}", in(reg) gs[cur].mair_el1);
                    }
                }
                {
                    let v: u64;
                    asm!("mrs {}, sctlr_el1", out(reg) v);
                    if v != gs[cur].sctlr_el1 {
                        asm!("msr sctlr_el1, {}", in(reg) gs[cur].sctlr_el1);
                    }
                }
                asm!("msr tpidr_el0, {}", in(reg) gs[cur].tpidr_el0);
                asm!("msr tpidr_el1, {}", in(reg) gs[cur].tpidr_el1);
                asm!("msr sp_el0, {}", in(reg) gs[cur].sp_el0);
                asm!("msr vbar_el1, {}", in(reg) gs[cur].vbar_el1);
                asm!("msr cpacr_el1, {}", in(reg) gs[cur].cpacr_el1);
                asm!("isb");
                asm!("msr cntv_cval_el0, {}", in(reg) gs[cur].cntv_cval);
                asm!("msr cntv_ctl_el0, {}", in(reg) gs[cur].cntv_ctl);
                asm!("msr cntp_tval_el0, {}", in(reg) SLICE_TICKS);
                let one: u64 = 1;
                asm!("msr cntp_ctl_el0, {}", in(reg) one);
                gs[cur].started = true;
                prev_cur = cur;
            }
            let hcr = if gs[cur].vgic.has_pending() {
                (HCR_EL2_BASE | HCR_VI) & !HCR_TWI
            } else {
                HCR_EL2_BASE
            };
            asm!("msr hcr_el2, {}", in(reg) hcr);
            asm!("isb");
        }
        let esr = unsafe { vcpu_enter(&mut gs[cur].vcpu as *mut Vcpu) };
        let uart_flags = unsafe { ((UART_BASE + 0x18) as *const u32).read_volatile() };
        if uart_flags & 0x10 == 0 {
            let input = unsafe { (UART_BASE as *const u8).read_volatile() };
            match console.input(input) {
                InputAction::Attach(guest) if guest < gs.len() && !gs[guest].done => {
                    console.attach(guest);
                }
                InputAction::Status => {
                    for (id, guest) in gs.iter().enumerate() {
                        kor::println!(
                            "guest {}: {} pc={:#x}",
                            id,
                            if guest.done { "stopped" } else { "running" },
                            guest.vcpu.elr
                        );
                    }
                    console.prompt();
                }
                InputAction::GuestByte { guest, byte } => {
                    gs[guest].vuart.rx_push(byte);
                    gs[guest].vgic.inject(33);
                }
                _ => {}
            }
        }
        let physical_irq = (esr >> 63) != 0;
        let guest_esr = esr & !(1u64 << 63);
        let ec = guest_esr >> 26;
        let iar = if physical_irq {
            unsafe { ((GICC_BASE + 0x0c) as *const u32).read_volatile() }
        } else {
            0x3ff
        };
        if physical_irq && iar < 0x3fe {
            if iar == VIRQ_TIMER as u32 {
                gs[cur].vgic.inject(VIRQ_TIMER);
                gs[cur].cntv_ctl = 0;
                unsafe {
                    asm!("msr cntv_ctl_el0, xzr");
                }
            } else if iar == PPI_CNTP as u32 {
                unsafe {
                    asm!("msr cntp_ctl_el0, xzr");
                }
            } else if iar >= 32 {
                // G1's device uses SPI 49 but guest expects IRQ 48 (shared DTB)
                let virq = if iar == 49 && cur == 1 { 48 } else { iar };
                gs[cur].vgic.inject(virq);
            }
            kor::PhysAddr::new((GICC_BASE + 0x10) as usize)
                .to_ka()
                .write_u32(iar); // EOI
            continue;
        }
        match ec {
            EC_SMC | EC_HVC => {
                if gs[cur].vcpu.regs[0] == 0x100 {
                    // HVC getchar: read directly from real UART, skip nulls, fallback to vUART
                    let fr = unsafe { ((UART_BASE + 0x18) as *const u32).read_volatile() };
                    if fr & 0x10 == 0 {
                        let ch = unsafe { (UART_BASE as *const u8).read_volatile() } as u64;
                        gs[cur].vcpu.regs[0] = if ch != 0 { ch } else { 0xffffffff };
                    } else {
                        let ch = gs[cur].vuart.rx_pop();
                        gs[cur].vcpu.regs[0] = match ch {
                            Some(c) if c != 0 => c as u64,
                            _ => 0xffffffff,
                        };
                    }
                } else if handle_psci(&mut gs[cur].vcpu) {
                    kor::println!("\n[el2: G{} PSCI done]", cur);
                    gs[cur].done = true;
                    cur = 1 - cur;
                }
            }
            EC_WFI => {
                // Guest is idle. Clear CNTP GIC pending so QEMU TCG halts CPU
                // and runs main loop (chardev reads stdin). Timer still runs,
                // will re-raise interrupt after SLICE_TICKS to wake CPU.
                // Poll real UART for input
                let fr = unsafe { ((UART_BASE + 0x18) as *const u32).read_volatile() };
                if fr & 0x10 == 0 {
                    let ch = unsafe { (UART_BASE as *const u8).read_volatile() };
                    if ch != 0 {
                        gs[cur].vuart.rx_push(ch);
                    }
                }
                gs[cur].vcpu.elr += 4;
                if !gs[1 - cur].done {
                    cur = 1 - cur;
                }
            }
            EC_IABT | EC_DABT => {
                let guest = &mut gs[cur];
                if !handle_mmio(
                    &mut guest.vcpu,
                    guest_esr,
                    &mut guest.vgic,
                    &mut guest.vuart,
                    cur,
                    &guest.memory,
                ) {
                    kor::println!("[el2: G{} MMIO fail]", cur);
                    guest.done = true;
                    cur = 1 - cur;
                }
            }
            _ => {
                let far: u64;
                unsafe {
                    asm!("mrs {}, far_el2", out(reg) far);
                }
                kor::println!(
                    "[el2: G{} trap ec={:#x} elr={:#x} far={:#x}]",
                    cur,
                    ec,
                    gs[cur].vcpu.elr,
                    far
                );
                gs[cur].done = true;
                cur = 1 - cur;
            }
        }
    }
    kor::println!("el2: all guests done");
}
