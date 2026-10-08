//! Minimal GICv2 state model for one guest.

const IRQ_WORDS: usize = 8;
const IRQ_COUNT: usize = 256;

pub struct VirtualGic {
    distributor_ctlr: u32,
    enabled: [u32; IRQ_WORDS],
    pending: [u32; IRQ_WORDS],
    priority: [u8; IRQ_COUNT],
    cpu_ctlr: u32,
    cpu_pmr: u32,
    active: u32,
}

impl VirtualGic {
    pub const fn new() -> Self {
        Self {
            distributor_ctlr: 0,
            enabled: [0; IRQ_WORDS],
            pending: [0; IRQ_WORDS],
            priority: [0; IRQ_COUNT],
            cpu_ctlr: 1,
            cpu_pmr: 0xff,
            active: 0x3ff,
        }
    }

    pub fn inject(&mut self, irq: u32) {
        self.pending[(irq / 32) as usize] |= 1 << (irq % 32);
    }

    pub fn has_pending(&self) -> bool {
        self.pending
            .iter()
            .zip(self.enabled.iter())
            .any(|(pending, enabled)| pending & enabled != 0)
    }

    pub fn iar(&mut self) -> u32 {
        for index in 0..IRQ_WORDS {
            let ready = self.pending[index] & self.enabled[index];
            if ready != 0 {
                let irq = index as u32 * 32 + ready.trailing_zeros();
                self.pending[index] &= !(1 << (irq % 32));
                self.active = irq;
                return irq;
            }
        }
        0x3ff
    }

    pub fn read_distributor(&self, offset: usize) -> u32 {
        match offset {
            0x000 => self.distributor_ctlr,
            0x004 => 7,
            0x100..=0x11c => self.enabled[(offset - 0x100) / 4],
            0x180..=0x19c => self.enabled[(offset - 0x180) / 4],
            0x200..=0x21c => self.pending[(offset - 0x200) / 4],
            0x280..=0x29c => self.pending[(offset - 0x280) / 4],
            0x400..=0x4ff => self.priority[offset - 0x400] as u32,
            0x800..=0x8ff => 1,
            _ => 0,
        }
    }

    pub fn write_distributor(&mut self, offset: usize, value: u32) {
        match offset {
            0x000 => self.distributor_ctlr = value,
            0x100..=0x11c => self.enabled[(offset - 0x100) / 4] |= value,
            0x180..=0x19c => self.enabled[(offset - 0x180) / 4] &= !value,
            0x200..=0x21c => self.pending[(offset - 0x200) / 4] |= value,
            0x280..=0x29c => self.pending[(offset - 0x280) / 4] &= !value,
            0x400..=0x4ff => self.priority[offset - 0x400] = value as u8,
            _ => {}
        }
    }

    pub fn read_cpu(&mut self, offset: usize) -> u32 {
        match offset {
            0x000 => self.cpu_ctlr,
            0x004 => self.cpu_pmr,
            0x00c => self.iar(),
            0x018 => self.pending_irq(),
            _ => 0,
        }
    }

    pub fn write_cpu(&mut self, offset: usize, value: u32) {
        match offset {
            0x000 => self.cpu_ctlr = value,
            0x004 => self.cpu_pmr = value,
            0x010 if self.active == value => self.active = 0x3ff,
            _ => {}
        }
    }

    fn pending_irq(&self) -> u32 {
        (0..IRQ_WORDS)
            .find_map(|index| {
                let ready = self.pending[index] & self.enabled[index];
                (ready != 0).then_some(index as u32 * 32 + ready.trailing_zeros())
            })
            .unwrap_or(0x3ff)
    }
}
