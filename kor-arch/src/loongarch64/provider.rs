//! `ArchProvider` implementation for loongarch64.
//!
//! Delegates to the per-arch functions under `arch/loongarch64/`.  `now_ticks`
//! reads the stable counter (`rdtime.d`) and `timer_hz` derives the constant
//! timer frequency from `cpucfg`, mirroring `time::timer_freq`.

use kor::{ArchProvider, MapError, MapSize, MappingFlags, TaskContext};
use core::arch::asm;

/// The loongarch64 architecture provider — a zero-sized marker.
pub struct Loongarch64Provider;

/// Singleton instance installed by the binary crate.
pub static PROVIDER: Loongarch64Provider = Loongarch64Provider;

// loongarch64 callee-saved context (ra + sp + fp + s[9] = 12 usizes) fits in 16.
const _: () =
    assert!(core::mem::size_of::<super::context::TaskContext>() <= core::mem::size_of::<TaskContext>());

impl ArchProvider for Loongarch64Provider {
    fn kernel_offset(&self) -> usize {
        super::mm::kernel_offset()
    }
    fn phys_to_virt(&self, pa: usize) -> usize {
        super::mm::phys_to_virt(pa)
    }
    fn virt_to_phys(&self, va: usize) -> usize {
        super::mm::virt_to_phys(va)
    }
    fn dtb_ptr(&self) -> usize {
        super::mm::dtb_ptr()
    }
    fn boot_cmdline(&self) -> Option<alloc::string::String> {
        super::mm::boot_cmdline()
    }
    fn detect_memory_regions(&self, add_region: &mut dyn FnMut(usize, usize)) {
        super::mm::init(|start, end| add_region(start, end));
    }

    fn page_table_init(&self) {
        super::page_table::init();
    }
    fn map(
        &self,
        vaddr: usize,
        paddr: usize,
        flags: MappingFlags,
        size: MapSize,
    ) -> Result<(), MapError> {
        super::page_table::map(vaddr, paddr, flags, size)
    }
    fn translate(&self, vaddr: usize) -> Option<usize> {
        super::page_table::translate(vaddr)
    }
    fn dynamic_maps_supported(&self) -> bool {
        super::page_table::dynamic_maps_supported()
    }

    fn trap_init(&self) {
        super::trap::init();
    }

    fn irq_enable(&self) {
        super::irq::enable();
    }
    fn irq_disable(&self) {
        super::irq::disable();
    }
    fn irq_is_enabled(&self) -> bool {
        super::irq::is_enabled()
    }

    fn timer_init(&self) {
        super::time::init();
    }
    fn handle_tick(&self) {
        super::time::handle_tick();
    }

    fn cpu_id(&self) -> usize {
        super::smp::cpu_id()
    }
    fn wait_for_interrupt(&self) {
        super::smp::wait_for_interrupt();
    }
    fn start_secondaries(&self) -> usize {
        super::smp::start_secondaries()
    }

    fn task_context_zero(&self) -> TaskContext {
        TaskContext { _storage: [0; 16] }
    }
    fn task_context_init(&self, ctx: &mut TaskContext, entry: usize, stack_top: usize) {
        // SAFETY: `super::context::TaskContext` (12 usizes) fits in the 16-usize
        // opaque buffer (checked above); both are `repr(C, align(16))`.
        let inner = unsafe { &mut *(ctx as *mut TaskContext as *mut super::context::TaskContext) };
        inner.init(entry, stack_top);
    }
    unsafe fn context_switch(&self, prev: *mut TaskContext, next: *const TaskContext) {
        // SAFETY: caller guarantees valid, non-aliasing, aligned buffers.
        unsafe {
            super::context::context_switch(
                prev as *mut super::context::TaskContext,
                next as *const super::context::TaskContext,
            )
        }
    }

    fn now_ticks(&self) -> u64 {
        // `rdtime.d` reads the 64-bit stable counter (wall clock).
        let t: u64;
        // SAFETY: `rdtime.d` is an unprivileged read of the stable counter.
        unsafe { asm!("rdtime.d {}, $zero", out(reg) t) };
        t
    }
    fn timer_hz(&self) -> u64 {
        // Constant-timer frequency from CPUCFG words 4 (base) and 5 (mul/div),
        // identical to `time::timer_freq`.
        let base: usize;
        let muldiv: usize;
        // SAFETY: CPUCFG is a read-only configuration instruction.
        unsafe {
            asm!("cpucfg {}, {}", out(reg) base, in(reg) 4usize);
            asm!("cpucfg {}, {}", out(reg) muldiv, in(reg) 5usize);
        }
        let cc_freq = (base & 0xffff_ffff) as u64;
        let mul = (muldiv & 0xffff) as u64;
        let div = ((muldiv >> 16) & 0xffff) as u64;
        if cc_freq == 0 || mul == 0 || div == 0 {
            100_000_000 // fallback
        } else {
            cc_freq * mul / div
        }
    }
}
