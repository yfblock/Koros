//! Minimal aarch64 ArchProvider for the korhv host (EL2, identity-mapped).
//! Only the methods needed by boot/heap/memory-init are real; scheduler/timer
//! methods are stubs (single-CPU, interrupts disabled).

use alloc::string::String;
use core::arch::asm;
use kor::{ArchProvider, MapError, MapSize, MappingFlags, TaskContext};

pub struct Aarch64Provider;
pub static PROVIDER: Aarch64Provider = Aarch64Provider;

impl ArchProvider for Aarch64Provider {
    fn kernel_offset(&self) -> usize {
        0
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
    fn boot_cmdline(&self) -> Option<String> {
        super::mm::boot_cmdline()
    }
    fn detect_memory_regions(&self, add_region: &mut dyn FnMut(usize, usize)) {
        super::mm::detect_memory_regions(add_region);
    }

    fn page_table_init(&self) {
        // The boot identity map (1 GiB blocks) suffices for the hypervisor.
    }
    fn map(
        &self,
        _vaddr: usize,
        _paddr: usize,
        _flags: MappingFlags,
        _size: MapSize,
    ) -> Result<(), MapError> {
        Err(MapError::Unsupported)
    }
    fn translate(&self, vaddr: usize) -> Option<usize> {
        // Identity-mapped low region.
        if vaddr < 0x1_0000_0000 {
            Some(vaddr)
        } else {
            None
        }
    }
    fn dynamic_maps_supported(&self) -> bool {
        false
    }

    fn trap_init(&self) {
        super::hyp::trap_init();
    }

    fn irq_enable(&self) {
        // SAFETY: unmask IRQ in DAIF (clear bit 7).
        unsafe { asm!("msr daifclr, #2") };
    }
    fn irq_disable(&self) {
        // SAFETY: mask IRQ in DAIF (set bit 7).
        unsafe { asm!("msr daifset, #2") };
    }
    fn irq_is_enabled(&self) -> bool {
        let daif: u64;
        // SAFETY: read DAIF.
        unsafe { asm!("mrs {}, daif", out(reg) daif) };
        daif & (1 << 7) == 0
    }

    fn timer_init(&self) {}
    fn handle_tick(&self) {}

    fn cpu_id(&self) -> usize {
        let mpidr: u64;
        // SAFETY: read MPIDR_EL1 (accessible at EL2).
        unsafe { asm!("mrs {}, mpidr_el1", out(reg) mpidr) };
        (mpidr & 0xff) as usize
    }
    fn wait_for_interrupt(&self) {
        // SAFETY: wfi is a hint.
        unsafe { asm!("wfi") };
    }
    fn start_secondaries(&self) -> usize {
        0
    }

    fn task_context_zero(&self) -> TaskContext {
        TaskContext { _storage: [0; 16] }
    }
    fn task_context_init(&self, _ctx: &mut TaskContext, _entry: usize, _stack_top: usize) {
        panic!("korhv does not use the scheduler");
    }
    unsafe fn context_switch(&self, _prev: *mut TaskContext, _next: *const TaskContext) {
        panic!("korhv does not use the scheduler");
    }

    fn now_ticks(&self) -> u64 {
        0
    }
    fn timer_hz(&self) -> u64 {
        100
    }
}
