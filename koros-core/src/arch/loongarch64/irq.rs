//! loongarch64 local interrupt control via `CRMD.IE` (bit 2).

use core::arch::asm;

const CRMD_IE: usize = 1 << 2;

fn read_crmd() -> usize {
    let v: usize;
    // SAFETY: reads the CRMD CSR.
    unsafe { asm!("csrrd {}, 0x0", out(reg) v) };
    v
}

fn write_crmd(v: usize) {
    // `csrwr` swaps rd with the CSR; the old value is discarded.
    // SAFETY: writes the CRMD CSR.
    unsafe { asm!("csrwr {}, 0x0", inout(reg) v => _) };
}

pub fn enable() {
    write_crmd(read_crmd() | CRMD_IE);
}

pub fn disable() {
    write_crmd(read_crmd() & !CRMD_IE);
}

pub fn is_enabled() -> bool {
    read_crmd() & CRMD_IE != 0
}
