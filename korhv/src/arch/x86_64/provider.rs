//! Minimal x86_64 `ArchProvider` for the korhv host.  Only the methods needed
//! by the boot/heap/memory-init sequence are real; the scheduler/timer/SMP
//! methods are stubs (the hypervisor runs single-CPU with interrupts off).

use alloc::string::String;
use core::arch::asm;
use kor::{ArchProvider, MapError, MapSize, MappingFlags, TaskContext};

pub struct X86_64Provider;
pub static PROVIDER: X86_64Provider = X86_64Provider;

const KERNEL_OFFSET: usize = 0xffff_8000_0000_0000;

impl ArchProvider for X86_64Provider {
    fn kernel_offset(&self) -> usize {
        KERNEL_OFFSET
    }
    fn phys_to_virt(&self, pa: usize) -> usize {
        super::mm::phys_to_virt(pa)
    }
    fn virt_to_phys(&self, va: usize) -> usize {
        super::mm::virt_to_phys(va)
    }
    fn dtb_ptr(&self) -> usize {
        0
    }
    fn boot_cmdline(&self) -> Option<String> {
        super::mm::boot_cmdline()
    }
    fn detect_memory_regions(&self, add_region: &mut dyn FnMut(usize, usize)) {
        super::mm::detect_memory_regions(add_region);
    }

    fn page_table_init(&self) {
        // The boot page tables (identity + high-half 2 MiB maps) suffice for
        // the hypervisor; no dynamic host page table is needed.
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
        if vaddr >= KERNEL_OFFSET {
            Some(vaddr - KERNEL_OFFSET)
        } else if vaddr < 0x1_0000_0000 {
            Some(vaddr) // identity-mapped low region
        } else {
            None
        }
    }
    fn dynamic_maps_supported(&self) -> bool {
        false
    }

    fn trap_init(&self) {
        super::trap::init();
    }

    fn irq_enable(&self) {
        unsafe { asm!("sti") };
    }
    fn irq_disable(&self) {
        unsafe { asm!("cli") };
    }
    fn irq_is_enabled(&self) -> bool {
        let f: u64;
        unsafe { asm!("pushfq", "pop {}", out(reg) f) };
        f & (1 << 9) != 0
    }

    fn timer_init(&self) {}
    fn handle_tick(&self) {}

    fn cpu_id(&self) -> usize {
        0
    }
    fn wait_for_interrupt(&self) {
        unsafe { asm!("hlt") };
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
