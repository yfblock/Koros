//! `ArchProvider` trait, opaque `TaskContext`, and the installed-provider
//! registry.  Shared code calls [`current`] instead of `cfg`-selecting a
//! per-arch module.

extern crate alloc;

use alloc::string::String;
use spin::Once;

use crate::mapping::{MapError, MapSize, MappingFlags};

/// Opaque callee-saved register storage, large enough for every supported arch.
///
/// Arch providers cast this buffer to their internal layout with
/// `&mut *(ctx as *mut _ as *mut ArchTaskContext)`.  16 `usize`s (128 bytes)
/// hold riscv64 (14), aarch64 (13), loongarch64 (12) and x86_64 (1).
#[repr(C, align(16))]
pub struct TaskContext {
    pub _storage: [usize; 16],
}

/// The architecture-specific service surface, installed once at boot.
pub trait ArchProvider: Send + Sync {
    // MM / address translation
    fn kernel_offset(&self) -> usize;
    fn phys_to_virt(&self, pa: usize) -> usize;
    fn virt_to_phys(&self, va: usize) -> usize;
    fn dtb_ptr(&self) -> usize;
    fn boot_cmdline(&self) -> Option<String>;
    fn detect_memory_regions(&self, add_region: &mut dyn FnMut(usize, usize));

    // Page table
    fn page_table_init(&self);
    fn map(&self, vaddr: usize, paddr: usize, flags: MappingFlags, size: MapSize) -> Result<(), MapError>;
    fn translate(&self, vaddr: usize) -> Option<usize>;
    fn dynamic_maps_supported(&self) -> bool;

    // Trap install
    fn trap_init(&self);

    // IRQ (CPU-level interrupt enable/disable)
    fn irq_enable(&self);
    fn irq_disable(&self);
    fn irq_is_enabled(&self) -> bool;

    // Timer
    fn timer_init(&self);
    fn handle_tick(&self);

    // SMP
    fn cpu_id(&self) -> usize;
    fn wait_for_interrupt(&self);
    fn start_secondaries(&self) -> usize;

    // Context switch
    fn task_context_zero(&self) -> TaskContext;
    fn task_context_init(&self, ctx: &mut TaskContext, entry: usize, stack_top: usize);
    /// # Safety
    /// `prev`/`next` must point to valid, properly-aligned `TaskContext`
    /// buffers owned by tasks that are not concurrently being switched.
    unsafe fn context_switch(&self, prev: *mut TaskContext, next: *const TaskContext);

    // Benchmark timer
    fn now_ticks(&self) -> u64;
    fn timer_hz(&self) -> u64;
}

static PROVIDER: Once<&'static dyn ArchProvider> = Once::new();

/// Install the architecture provider.  Call once, first thing in `kernel_main`.
pub fn install(p: &'static dyn ArchProvider) {
    PROVIDER.call_once(|| p);
}

/// The installed architecture provider.  Panics if [`install`] was not called.
pub fn current() -> &'static dyn ArchProvider {
    PROVIDER.get().copied().expect("ArchProvider not installed")
}

/// Physical -> kernel direct-map virtual address, via the installed provider.
#[inline]
pub fn phys_to_virt(pa: usize) -> usize {
    current().phys_to_virt(pa)
}

/// Inverse of [`phys_to_virt`], via the installed provider.
#[inline]
pub fn virt_to_phys(va: usize) -> usize {
    current().virt_to_phys(va)
}
