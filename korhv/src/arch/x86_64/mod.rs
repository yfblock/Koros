//! x86_64 arch layer for korhv.

pub mod boot;
pub mod console;
pub mod hyp;
pub mod mm;
pub mod provider;
pub mod trap;

pub fn platform_config() -> kor::config::PlatformConfig {
    kor::config::PlatformConfig {
        console: kor::config::Console::Ns16550aPort { base: 0x3f8 },
        firmware_phys_start: 0,
        dtb: 0,
        pci: None,
    }
}

pub fn add_free_region(start: usize, end: usize) {
    kor_frame::add_region(start, end);
}

struct Runtime;

impl crate::hypervisor::Hypervisor for Runtime {
    fn init(&self) {
        hyp::init();
    }

    fn run(&self) {
        hyp::run();
    }
}

static RUNTIME: Runtime = Runtime;

pub fn hypervisor() -> &'static dyn crate::hypervisor::Hypervisor {
    &RUNTIME
}
