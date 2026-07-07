//! `ArchProvider` implementation for x86_64.
//!
//! Delegates to the per-arch functions under `arch/x86_64/`.  `now_ticks`
//! reads the TSC and `timer_hz` calibrates it against the 8254 PIT, matching
//! the existing `bench` module so benchmark numbers are unchanged.

use kor::{ArchProvider, MapError, MapSize, MappingFlags, TaskContext};
use x86_64::instructions::port::Port;

/// The x86_64 architecture provider — a zero-sized marker.
pub struct X86_64Provider;

/// Singleton instance installed by the binary crate.
pub static PROVIDER: X86_64Provider = X86_64Provider;

// x86_64 callee-saved context (just `rsp` = 1 usize) fits trivially.
const _: () =
    assert!(core::mem::size_of::<super::context::TaskContext>() <= core::mem::size_of::<TaskContext>());

impl ArchProvider for X86_64Provider {
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
        // SAFETY: `super::context::TaskContext` (1 usize) fits in the 16-usize
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
        // SAFETY: `rdtsc` reads the time-stamp counter; safe in ring 0.
        unsafe { core::arch::x86_64::_rdtsc() }
    }
    fn timer_hz(&self) -> u64 {
        // Calibrate the TSC against the 1.193182 MHz PIT (channel 2), gated so
        // it needs no interrupt — same logic as `bench::timer_hz`.
        const PIT_HZ: u64 = 1_193_182;
        let count: u16 = 11_932; // ≈ 10 ms
        // SAFETY: ring-0 port I/O to the legacy PIT and control port.
        unsafe {
            let mut p61 = Port::<u8>::new(0x61);
            let orig = p61.read();
            p61.write((orig & 0xFC) | 0x01);
            Port::<u8>::new(0x43).write(0b1011_0000);
            Port::<u8>::new(0x42).write((count & 0xFF) as u8);
            Port::<u8>::new(0x42).write((count >> 8) as u8);

            let start = self.now_ticks();
            while p61.read() & 0x20 == 0 {}
            let end = self.now_ticks();
            p61.write(orig);

            (end - start).saturating_mul(PIT_HZ) / count as u64
        }
    }
}
