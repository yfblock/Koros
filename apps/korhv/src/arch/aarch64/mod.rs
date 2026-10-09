//! AArch64 architecture layer for Korhv.

pub mod boot;
pub mod console;
pub mod console_mux;
pub mod guest_image;
pub mod hyp;
pub mod mm;

const GUEST_RESERVED_START: usize = 0x4020_0000;
const GUEST_RESERVED_END: usize = 0x5000_0000;

pub fn platform_config() -> kor::config::PlatformConfig {
    kor::config::PlatformConfig {
        firmware_phys_start: 0x4000_0000,
        dtb: 0,
        pci: None,
    }
}

pub fn add_free_region(start: usize, end: usize) {
    kor::regions::clip_region(
        start,
        end,
        GUEST_RESERVED_START,
        GUEST_RESERVED_END,
        kor::frame::add_region,
    );
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
