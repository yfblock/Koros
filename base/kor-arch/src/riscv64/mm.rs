use alloc::string::String;
use core::sync::atomic::{AtomicUsize, Ordering};

pub fn kernel_offset() -> usize {
    0xffff_ffc0_0000_0000
}

/// Read the kernel command line from the device tree `/chosen/bootargs`.
pub fn boot_cmdline() -> Option<String> {
    let dtb = DTB_PTR.load(Ordering::Relaxed);
    if dtb == 0 {
        return None;
    }
    // SAFETY: `DTB_PTR` was captured at boot and points to a valid FDT blob.
    unsafe { kor::fdt::bootargs(dtb) }
}

pub fn phys_to_virt(pa: usize) -> usize {
    pa + kernel_offset()
}

pub fn virt_to_phys(va: usize) -> usize {
    va - kernel_offset()
}

pub(crate) static DTB_PTR: AtomicUsize = AtomicUsize::new(0);

pub fn set_dtb_ptr(dtb: usize) {
    // Reject non-FDT garbage (e.g. U-Boot `go` passes argv in a1): the
    // blob must start with the FDT magic big-endian word.
    if fdt_magic_at(dtb) {
        DTB_PTR.store(dtb, Ordering::Relaxed);
        return;
    }
    // `go` passes argv in a1 (not FDT).  The boot script stashes the
    // firmware's control-FDT address at this well-known mailbox address
    // before jumping (see the SG2002 boardctl .scr: `mw.l {addr} $fdtcontroladdr`).
    const FDT_MAILBOX: usize = 0x801F_F000;
    // The boot script stores the firmware's control-FDT *pointer* here.
    // SAFETY: one aligned read from the fixed mailbox address.
    let candidate = unsafe { (FDT_MAILBOX as *const usize).read_volatile() };
    if fdt_magic_at(candidate) {
        DTB_PTR.store(candidate, Ordering::Relaxed);
        return;
    }
    DTB_PTR.store(0, Ordering::Relaxed);
}

/// Check for the FDT magic (0xD00DFEED big-endian) at `addr`.
fn fdt_magic_at(addr: usize) -> bool {
    addr != 0
        // SAFETY: one 32-bit aligned read.
        && unsafe { (addr as *const u32).read_volatile() }.to_be() == 0xd00d_feed
}

/// Pointer to the flattened device tree, or 0 if none was provided.
pub fn dtb_ptr() -> usize {
    DTB_PTR.load(Ordering::Relaxed)
}

/// Detect physical memory regions via FDT and register them with `add_region`.
pub fn init(mut add_region: impl FnMut(usize, usize)) {
    let dtb = DTB_PTR.load(Ordering::Relaxed);
    if dtb != 0 {
        unsafe {
            kor::fdt::parse_memory_regions(dtb, &mut add_region);
        }
    }
}
