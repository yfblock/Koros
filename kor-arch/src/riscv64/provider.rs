//! `ArchProvider` implementation for riscv64.
//!
//! A zero-sized static (`PROVIDER`) implements [`kor::ArchProvider`] by
//! delegating each method to the existing per-arch functions under
//! `arch/riscv64/`.  The binary crate installs it once in `kernel_main`.

use kor::{ArchProvider, MapError, MapSize, MappingFlags, TaskContext};
use core::arch::asm;

/// The riscv64 architecture provider — a zero-sized marker.
pub struct Riscv64Provider;

/// Singleton instance installed by the binary crate.
pub static PROVIDER: Riscv64Provider = Riscv64Provider;

// riscv64 callee-saved context (ra + sp + s[12] = 14 usizes) fits in the
// 16-usize opaque `hal::TaskContext` buffer.
const _: () =
    assert!(core::mem::size_of::<super::context::TaskContext>() <= core::mem::size_of::<TaskContext>());

impl ArchProvider for Riscv64Provider {
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
        // SAFETY: `super::context::TaskContext` (14 usizes) fits in the 16-usize
        // opaque buffer (checked above), and both are `repr(C, align(16))`.
        let inner = unsafe { &mut *(ctx as *mut TaskContext as *mut super::context::TaskContext) };
        inner.init(entry, stack_top);
    }
    unsafe fn context_switch(&self, prev: *mut TaskContext, next: *const TaskContext) {
        // SAFETY: caller guarantees valid, non-aliasing, aligned buffers; the
        // cast is sound because the arch context fits in the opaque buffer.
        unsafe {
            super::context::context_switch(
                prev as *mut super::context::TaskContext,
                next as *const super::context::TaskContext,
            )
        }
    }

    fn now_ticks(&self) -> u64 {
        // `rdtime` is the QEMU `virt` 10 MHz wall-clock counter (matches
        // `timer_hz`); using it keeps benchmark numbers consistent with the
        // tick frequency.
        let t: u64;
        unsafe { asm!("rdtime {}", out(reg) t) };
        t
    }
    fn timer_hz(&self) -> u64 {
        10_000_000
    }
}
