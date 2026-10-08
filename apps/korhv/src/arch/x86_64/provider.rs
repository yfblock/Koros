//! Minimal x86_64 `ArchProvider` for the korhv host.  Only the methods needed
//! by the boot/heap/memory-init sequence are real; the scheduler/timer/SMP
//! methods are stubs (the hypervisor runs single-CPU with interrupts off).

use alloc::string::String;
use core::arch::asm;
use kor::{ArchProvider, TaskContext};

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

    fn shutdown(&self) -> ! {
        // QEMU poweroff: SLP_EN|SLP_TYP=0 through the ACPI PM1a register,
        // then the legacy port, then park.
        const QEMU_PM1A_CNT: u16 = 0x604;
        const QEMU_PM1A_CNT_LEGACY: u16 = 0xB004;
        const PM1_SLP_EN: u16 = 1 << 13;
        // SAFETY: port I/O writes and `hlt`, valid in PLV0.
        unsafe {
            asm!("out dx, ax", in("dx") QEMU_PM1A_CNT, in("ax") PM1_SLP_EN);
            asm!("out dx, ax", in("dx") QEMU_PM1A_CNT_LEGACY, in("ax") PM1_SLP_EN);
        }
        loop {
            // SAFETY: `hlt` halts until the next interrupt.
            unsafe { asm!("hlt") };
        }
    }

    fn task_context_zero(&self) -> TaskContext {
        TaskContext { _storage: [0; 32] }
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
