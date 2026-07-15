//! AMD SVM hypervisor for x86_64.
//!
//! Enables SVM (EFER.SVME + host save area), builds a stage-2 (NPT) identity
//! map, sets up a VMCB for a 32-bit protected-mode guest, and runs a vCPU in a
//! VMRUN/#VMEXIT loop.  The guest communicates via `VMMCALL` hypercalls:
//!   rax = HCALL_PUTCHAR (1), rdi = char   -> host prints the char
//!   rax = HCALL_EXIT   (2), rdi = code    -> host stops the vCPU
//! On #VMEXIT, guest RAX is in the VMCB save area; guest RDI is left in the
//! RDI register (SVM saves only RAX to the VMCB automatically).

use core::arch::{asm, global_asm, naked_asm};

// --- MSRs / EFER -----------------------------------------------------------
const MSR_EFER: u32 = 0xC000_0080;
const EFER_SVME: u64 = 1 << 12;
const MSR_VM_HSAVE_PA: u32 = 0xC001_0117;

// --- SVM exit codes --------------------------------------------------------
const VMEXIT_HLT: u64 = 0x078;
const VMEXIT_VMMCALL: u64 = 0x081;
const VMEXIT_NPF: u64 = 0x400;
const VMEXIT_ERR: u64 = u64::MAX;

// --- VMCB control-area offsets --------------------------------------------
const INTR_W3: usize = 0x00C; // HLT is bit 24 here
const INTR_W4: usize = 0x010; // VMMCALL is bit 1 here
const ASID: usize = 0x058;
const TLB_CTL: usize = 0x05C;
const INT_CTL: usize = 0x060;
const EXIT_CODE: usize = 0x070;
const NESTED_CTL: usize = 0x090; // NP control: bit0 = SVM_NPT_ENABLED
const EXIT_INFO1: usize = 0x078;
const EXIT_INFO2: usize = 0x080;
const NESTED_CR3: usize = 0x0B0; // NP control: bit0 = NP_ENABLE, bits[51:12] = N_CR3

// --- VMCB state-save-area offsets (absolute, control area = 0x400) --------
const SAVE: usize = 0x400;
const O_ES: usize = SAVE + 0x000;
const O_CS: usize = SAVE + 0x010;
const O_SS: usize = SAVE + 0x020;
const O_DS: usize = SAVE + 0x030;
const O_FS: usize = SAVE + 0x040;
const O_GS: usize = SAVE + 0x050;
const O_GDTR: usize = SAVE + 0x060;
const O_LDTR: usize = SAVE + 0x070;
const O_IDTR: usize = SAVE + 0x080;
const O_TR: usize = SAVE + 0x090;
const O_CPL: usize = SAVE + 0x0CB;
const O_EFER: usize = SAVE + 0x0D0;
const O_CR4: usize = SAVE + 0x148;
const O_CR3: usize = SAVE + 0x150;
const O_CR0: usize = SAVE + 0x158;
const O_DR6: usize = SAVE + 0x168;
const O_DR7: usize = SAVE + 0x160;
const O_RFLAGS: usize = SAVE + 0x170;
const O_RIP: usize = SAVE + 0x178;
const O_RSP: usize = SAVE + 0x1D8;
const O_RAX: usize = SAVE + 0x1F8;
const O_G_PAT: usize = SAVE + 0x268;

const NP_ENABLE: u64 = 1;

// Segment attribute helpers (12-bit: G|D/B|L|AVL|P|DPL|S|type).
const ATTR_CS32: u16 = 0xC9B; // G,D, P, S, code exec/read, accessed
const ATTR_DS: u16 = 0xC93; // G,D, P, S, data read/write, accessed
const ATTR_TR: u16 = 0x08B; // P, system, 32-bit TSS busy
const ATTR_LDTR: u16 = 0x082; // P, system, LDT

extern crate alloc;

unsafe fn w64(p: *mut u8, off: usize, v: u64) {
    (p.add(off) as *mut u64).write_volatile(v);
}
unsafe fn w32(p: *mut u8, off: usize, v: u32) {
    (p.add(off) as *mut u32).write_volatile(v);
}
unsafe fn w16(p: *mut u8, off: usize, v: u16) {
    (p.add(off) as *mut u16).write_volatile(v);
}
unsafe fn w8(p: *mut u8, off: usize, v: u8) {
    (p.add(off) as *mut u8).write_volatile(v);
}
unsafe fn r64(p: *const u8, off: usize) -> u64 {
    (p.add(off) as *const u64).read_volatile()
}

unsafe fn set_seg(p: *mut u8, off: usize, sel: u16, attr: u16, limit: u32, base: u64) {
    w16(p, off, sel);
    w16(p, off + 2, attr);
    w32(p, off + 4, limit);
    w64(p, off + 8, base);
}

/// Enable SVM: set EFER.SVME and install a host save area.
pub fn init() {
    // SAFETY: wrmsr/rdmsr on EFER and VM_HSAVE_PA.  The host save area is a
    // freshly allocated 4 KiB frame; its physical address is what the CPU
    // saves/restores host state to across VMRUN/#VMEXIT.
    let hsave_phys = kor_frame::alloc_page().expect("alloc host save area");
    unsafe {
        wrmsr(MSR_VM_HSAVE_PA, hsave_phys as u64);
        let efer = rdmsr(MSR_EFER);
        wrmsr(MSR_EFER, efer | EFER_SVME);
    }
    kor::println!("svm: enabled (EFER.SVME=1, hsave={:#x})", hsave_phys);
}

unsafe fn wrmsr(msr: u32, val: u64) {
    asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") (val & 0xFFFF_FFFF) as u32,
        in("edx") (val >> 32) as u32,
    );
}
unsafe fn rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi);
    ((hi as u64) << 32) | (lo as u64)
}

const GUEST_GPA_BASE: usize = 0;
const GUEST_FRAMES: usize = 512;
const GUEST_SIZE: usize = GUEST_FRAMES * kor_frame::PAGE_SIZE;

struct GuestMemory {
    host_phys: usize,
}

impl GuestMemory {
    fn allocate() -> Self {
        let host_phys = kor_frame::alloc_frames(GUEST_FRAMES).expect("guest ram");
        assert_eq!(
            host_phys & (GUEST_SIZE - 1),
            0,
            "guest RAM must be 2 MiB aligned"
        );
        Self { host_phys }
    }

    fn host_ptr(&self) -> *mut u8 {
        kor::arch::current().phys_to_virt(self.host_phys) as *mut u8
    }
}

impl Drop for GuestMemory {
    fn drop(&mut self) {
        kor_frame::free_frames(self.host_phys, GUEST_FRAMES);
    }
}

struct NptRoot {
    pml4: usize,
    pdpt: usize,
    pd: usize,
}

impl NptRoot {
    /// Map only GPA 0..2 MiB to the frames owned by `memory`.
    fn new(memory: &GuestMemory) -> Self {
        let pml4 = kor_frame::alloc_page().expect("npt pml4");
        let pdpt = kor_frame::alloc_page().expect("npt pdpt");
        let pd = kor_frame::alloc_page().expect("npt pd");
        let pml4_v = kor::arch::current().phys_to_virt(pml4) as *mut u64;
        let pdpt_v = kor::arch::current().phys_to_virt(pdpt) as *mut u64;
        let pd_v = kor::arch::current().phys_to_virt(pd) as *mut u64;
        unsafe {
            core::ptr::write_bytes(pml4_v.cast::<u8>(), 0, 4096);
            core::ptr::write_bytes(pdpt_v.cast::<u8>(), 0, 4096);
            core::ptr::write_bytes(pd_v.cast::<u8>(), 0, 4096);
            pml4_v.write_volatile((pdpt as u64) | 0x07);
            pdpt_v.write_volatile((pd as u64) | 0x07);
            // P|RW|US|PS. Every other GPA remains non-present.
            pd_v.write_volatile((memory.host_phys as u64) | 0x87);
        }
        Self { pml4, pdpt, pd }
    }

    fn root(&self) -> usize {
        self.pml4
    }
}

impl Drop for NptRoot {
    fn drop(&mut self) {
        kor_frame::free_frames(self.pd, 1);
        kor_frame::free_frames(self.pdpt, 1);
        kor_frame::free_frames(self.pml4, 1);
    }
}
#[repr(C)]
struct SvmGprs {
    rcx: u64,
    rdx: u64,
    rbx: u64,
    rbp: u64,
    rsi: u64,
    rdi: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
}

const _: () = assert!(core::mem::size_of::<SvmGprs>() == 14 * 8);

/// save area.
#[unsafe(naked)]
#[unsafe(no_mangle)]
unsafe extern "C" fn svm_vmrun(vmcb_phys: u64, gprs: *mut SvmGprs) {
    naked_asm!(
        // Preserve host ABI state and retain the save-area pointer on the
        // host stack restored by SVM on every VMEXIT.
        "push rbx",
        "push rbp",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "push rsi",
        "mov rax, rdi",
        "mov rcx, [rsi + 0]",
        "mov rdx, [rsi + 8]",
        "mov rbx, [rsi + 16]",
        "mov rbp, [rsi + 24]",
        "mov rdi, [rsi + 40]",
        "mov r8, [rsi + 48]",
        "mov r9, [rsi + 56]",
        "mov r10, [rsi + 64]",
        "mov r11, [rsi + 72]",
        "mov r12, [rsi + 80]",
        "mov r13, [rsi + 88]",
        "mov r14, [rsi + 96]",
        "mov r15, [rsi + 104]",
        "mov rsi, [rsi + 32]",
        "vmrun",
        // RAX is transferred through the VMCB; all other guest GPRs remain in
        // registers and are copied to the explicit vCPU state before reuse.
        "mov rax, [rsp]",
        "mov [rax + 0], rcx",
        "mov [rax + 8], rdx",
        "mov [rax + 16], rbx",
        "mov [rax + 24], rbp",
        "mov [rax + 32], rsi",
        "mov [rax + 40], rdi",
        "mov [rax + 48], r8",
        "mov [rax + 56], r9",
        "mov [rax + 64], r10",
        "mov [rax + 72], r11",
        "mov [rax + 80], r12",
        "mov [rax + 88], r13",
        "mov [rax + 96], r14",
        "mov [rax + 104], r15",
        "add rsp, 8",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbp",
        "pop rbx",
        "ret",
    );
}

// Demo guest image: 32-bit protected-mode code.  Prints "Hi\n" via PUTCHAR
// hypercalls, then EXIT.  Assembled into .rodata; copied into guest RAM.
global_asm!(
    ".section .rodata",
    ".balign 16",
    ".global guest_img_start",
    "guest_img_start:",
    ".code32",
    "mov eax, 1",
    "mov edi, 0x48", // 'H'
    "vmmcall",
    "mov eax, 1",
    "mov edi, 0x69", // 'i'
    "vmmcall",
    "mov eax, 1",
    "mov edi, 0x21", // '!'
    "vmmcall",
    "mov eax, 2",
    "xor edi, edi",
    "vmmcall",
    "hlt",
    ".global guest_img_end",
    "guest_img_end:",
    ".code64",
);

unsafe extern "C" {
    static guest_img_start: u8;
    static guest_img_end: u8;
}

/// Create the VM, run its vCPU, handle hypercall exits until HCALL_EXIT.
pub fn run() {
    let arch = kor::arch::current();

    // 1. Guest RAM is an owned 2 MiB extent; the guest sees it at GPA zero.
    let guest_memory = GuestMemory::allocate();
    let guest_v = guest_memory.host_ptr();

    // 2. Copy the guest image to the start of guest RAM.
    let img_len = unsafe {
        let s = core::ptr::addr_of!(guest_img_start) as usize;
        let e = core::ptr::addr_of!(guest_img_end) as usize;
        e - s
    };
    unsafe {
        core::ptr::copy_nonoverlapping(core::ptr::addr_of!(guest_img_start), guest_v, img_len);
    }

    // 3. NPT maps only the owned guest extent.
    let npt = NptRoot::new(&guest_memory);

    // 4. VMCB (one 4 KiB frame).
    let vmcb_phys = kor_frame::alloc_page().expect("vmcb");
    let vmcb = arch.phys_to_virt(vmcb_phys) as *mut u8;
    unsafe {
        // Zero the whole VMCB.
        core::ptr::write_bytes(vmcb, 0, 4096);
        w32(vmcb, INTR_W4, (1 << 0) | (1 << 1)); // VMRUN (bit32) + VMMCALL (bit33)
        w32(vmcb, INTR_W3, 1 << 24);

        // ASID = 1, TLB_CTL = 0.
        w32(vmcb, ASID, 1);
        w8(vmcb, TLB_CTL, 0);
        w32(vmcb, INT_CTL, 0);

        // Nested paging: NP_ENABLE in nested_ctl, N_CR3 = NPT root.
        w64(vmcb, NESTED_CTL, NP_ENABLE);
        w64(vmcb, NESTED_CR3, npt.root() as u64);

        // --- Guest state: 32-bit protected mode, paging off ---
        // Segments (selector/attr/limit/base).
        set_seg(vmcb, O_CS, 0x08, ATTR_CS32, 0xFFFF_FFFF, 0);
        set_seg(vmcb, O_SS, 0x10, ATTR_DS, 0xFFFF_FFFF, 0);
        set_seg(vmcb, O_DS, 0x10, ATTR_DS, 0xFFFF_FFFF, 0);
        set_seg(vmcb, O_ES, 0x10, ATTR_DS, 0xFFFF_FFFF, 0);
        set_seg(vmcb, O_FS, 0x00, 0x0, 0, 0); // unusable
        set_seg(vmcb, O_GS, 0x00, 0x0, 0, 0); // unusable
        set_seg(vmcb, O_GDTR, 0x00, 0x0, 0xFFFF, 0);
        set_seg(vmcb, O_LDTR, 0x00, 0x0, 0, 0); // unusable
        set_seg(vmcb, O_IDTR, 0x00, 0x0, 0xFFFF, 0);
        set_seg(vmcb, O_TR, 0x00, ATTR_TR, 0xFFFF, 0);

        w8(vmcb, O_CPL, 0);
        w64(vmcb, O_EFER, EFER_SVME); // QEMU requires guest EFER.SVME set
        w64(vmcb, O_CR0, 0x11); // PE | ET
        w64(vmcb, O_CR3, 0);
        w64(vmcb, O_CR4, 0);
        w64(vmcb, O_DR6, 0);
        w64(vmcb, O_DR7, 0);
        w64(vmcb, O_RFLAGS, 0x2);
        w64(vmcb, O_G_PAT, 0x0007_0406_0007_0406);
        w64(vmcb, O_RIP, GUEST_GPA_BASE as u64);
        w64(vmcb, O_RSP, (GUEST_GPA_BASE + GUEST_SIZE - 0x100) as u64);
        w64(vmcb, O_RAX, 0);
    }

    kor::println!(
        "svm: vmcb={:#x} npt={:#x} guest_hpa={:#x} ({:#x} bytes), GPA entry=0",
        vmcb_phys,
        npt.root(),
        guest_memory.host_phys,
        img_len
    );

    // 5. VMRUN / #VMEXIT loop with explicit ownership of every non-VMCB GPR.
    let mut gprs = SvmGprs {
        rcx: 0,
        rdx: 0,
        rbx: 0,
        rbp: 0,
        rsi: 0,
        rdi: 0,
        r8: 0,
        r9: 0,
        r10: 0,
        r11: 0,
        r12: 0,
        r13: 0,
        r14: 0,
        r15: 0,
    };
    loop {
        unsafe { svm_vmrun(vmcb_phys as u64, &mut gprs) };
        let exit_code = unsafe { r64(vmcb, EXIT_CODE) };
        let guest_rdi = gprs.rdi;
        let guest_rax = unsafe { r64(vmcb, O_RAX) };

        if exit_code == VMEXIT_ERR {
            kor::println!("svm: VMRUN rejected guest state (VMEXIT_ERR)");
            break;
        }
        match exit_code {
            VMEXIT_VMMCALL => {
                match guest_rax as usize {
                    crate::HCALL_PUTCHAR => {
                        kor::print!("{}", guest_rdi as u8 as char);
                    }
                    crate::HCALL_EXIT => {
                        kor::println!("");
                        kor::println!("svm: guest EXIT(code={})", guest_rdi);
                        break;
                    }
                    n => {
                        kor::println!("\nsvm: unknown hypercall {}", n);
                        break;
                    }
                }
                // Advance RIP past VMMCALL (3 bytes; QEMU does not set next_rip).
                let rip = unsafe { r64(vmcb, O_RIP) };
                unsafe { w64(vmcb, O_RIP, rip + 3) };
            }
            VMEXIT_HLT => {
                kor::println!("\nsvm: guest HLT");
            }
            VMEXIT_NPF => {
                let info1 = unsafe { r64(vmcb, EXIT_INFO1) };
                let info2 = unsafe { r64(vmcb, EXIT_INFO2) };
                kor::println!(
                    "KORHV: unexpected NPF info1={:#x} info2={:#x}",
                    info1,
                    info2
                );
                break;
            }
            other => {
                let rip = unsafe { r64(vmcb, O_RIP) };
                kor::println!("\nsvm: unexpected exit {:#x} rip={:#x}", other, rip);
                break;
            }
        }
    }
    kor::println!("svm: vCPU run loop done");
}
