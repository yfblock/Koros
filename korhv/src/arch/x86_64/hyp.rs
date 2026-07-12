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

/// Build a 3-level NPT (PML4 -> PDPT -> PD) identity-mapping GPA [0, 1 GiB) to
/// HPA [0, 1 GiB) using 2 MiB pages.  Returns the PML4 physical address.
fn build_npt() -> usize {
    let pml4 = kor_frame::alloc_page().expect("npt pml4");
    let pdpt = kor_frame::alloc_page().expect("npt pdpt");
    let pd = kor_frame::alloc_page().expect("npt pd");
    let pml4_v = kor::arch::current().phys_to_virt(pml4) as *mut u64;
    let pdpt_v = kor::arch::current().phys_to_virt(pdpt) as *mut u64;
    let pd_v = kor::arch::current().phys_to_virt(pd) as *mut u64;
    unsafe {
        // PML4[0] -> PDPT (P|RW|US)
        pml4_v.add(0).write_volatile((pdpt as u64) | 0x07);
        // PDPT[0] -> PD (P|RW|US)
        pdpt_v.add(0).write_volatile((pd as u64) | 0x07);
        // PD[i] = (i*2MiB) | 0x87  (P|RW|US|PS, 2 MiB leaf)
        for i in 0..512u64 {
            pd_v.add(i as usize).write_volatile((i * 0x20_0000) | 0x87);
        }
    }
    pml4
}
/// save area.
#[unsafe(naked)]
#[unsafe(no_mangle)]
unsafe extern "C" fn svm_vmrun(vmcb_phys: u64, out: *mut u64) {
    naked_asm!(
        "push rbx", "push rbp", "push r12", "push r13", "push r14", "push r15",
        "push rsi",                 // save out ptr
        "mov rax, rdi",             // rax = VMCB phys for VMRUN
        "vmrun",
        // After #VMEXIT the host save area restored host RAX/RIP/RSP/etc.
        // GP regs other than RAX still hold guest values: capture guest RDI.
        "mov rcx, [rsp]",           // out ptr
        "mov [rcx], rdi",           // out[0] = guest rdi (hypercall arg0)
        "add rsp, 8",
        "pop r15", "pop r14", "pop r13", "pop r12", "pop rbp", "pop rbx",
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
    "mov edi, 0x48",       // 'H'
    "vmmcall",
    "mov eax, 1",
    "mov edi, 0x69",       // 'i'
    "vmmcall",
    "mov eax, 1",
    "mov edi, 0x21",       // '!'
    "vmmcall",
    "mov eax, 1",
    "mov edi, 0x0a",       // newline
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

    // 1. Guest RAM: 2 MiB contiguous (power-of-two frames).
    let guest_phys = kor_frame::alloc_frames(512).expect("guest ram"); // 2 MiB
    let guest_size: usize = 512 * 4096;
    let guest_v = arch.phys_to_virt(guest_phys) as *mut u8;

    // 2. Copy the guest image to the start of guest RAM.
    let img_len = unsafe {
        let s = core::ptr::addr_of!(guest_img_start) as usize;
        let e = core::ptr::addr_of!(guest_img_end) as usize;
        e - s
    };
    unsafe {
        core::ptr::copy_nonoverlapping(
            core::ptr::addr_of!(guest_img_start),
            guest_v,
            img_len,
        );
    }

    // 3. Stage-2 (NPT) identity map.
    let npt_root = build_npt();

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
        w64(vmcb, NESTED_CR3, npt_root as u64);

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
        w64(vmcb, O_RIP, guest_phys as u64); // entry = guest RAM base GPA
        w64(vmcb, O_RSP, (guest_phys + guest_size - 0x100) as u64);
        w64(vmcb, O_RAX, 0);
    }

    kor::println!(
        "svm: vmcb={:#x} npt={:#x} guest={:#x} ({:#x} bytes), entry rip={:#x}",
        vmcb_phys, npt_root, guest_phys, img_len, guest_phys
    );

    // 5. VMRUN / #VMEXIT loop.
    let mut out = [0u64; 1];
    loop {
        // SAFETY: svm_vmrun enters the guest; on #VMEXIT the guest's rdi is
        // captured in out[0].  The exit code and guest rax live in the VMCB.
        unsafe { svm_vmrun(vmcb_phys as u64, out.as_mut_ptr()) };
        let exit_code = unsafe { r64(vmcb, EXIT_CODE) };
        let guest_rdi = out[0];
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
                kor::println!("\nsvm: nested page fault (info1={:#x} info2={:#x})", info1, info2);
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
