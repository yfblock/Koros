//! PL011 state emulated for one AArch64 guest.

const RX_CAPACITY: usize = 256;
const RX_INDEX_MASK: usize = RX_CAPACITY - 1;
const UART_RX_IRQ: u32 = 33;

pub struct VirtualUart {
    cr: u32,
    lcr_h: u32,
    imsc: u32,
    ibrd: u32,
    ifls: u32,
    txris: bool,
    rxbuf: [u8; RX_CAPACITY],
    rxlen: usize,
    rxhead: usize,
}

impl VirtualUart {
    pub const fn new() -> Self {
        Self {
            cr: 0,
            lcr_h: 0,
            imsc: 0,
            ibrd: 0,
            ifls: 0,
            txris: false,
            rxbuf: [0; RX_CAPACITY],
            rxlen: 0,
            rxhead: 0,
        }
    }

    pub fn push_rx(&mut self, ch: u8) {
        if self.rxlen < RX_CAPACITY {
            let tail = (self.rxhead + self.rxlen) & RX_INDEX_MASK;
            self.rxbuf[tail] = ch;
            self.rxlen += 1;
        }
    }

    pub fn pop_rx(&mut self) -> Option<u8> {
        if self.rxlen == 0 {
            return None;
        }
        let ch = self.rxbuf[self.rxhead];
        self.rxhead = (self.rxhead + 1) & RX_INDEX_MASK;
        self.rxlen -= 1;
        Some(ch)
    }

    pub fn read(&mut self, offset: usize) -> u32 {
        match offset {
            0x000 => self.pop_rx().map(u32::from).unwrap_or(0),
            0x004 | 0x024 | 0x040 | 0x048 => 0,
            0x018 => {
                if self.has_rx() {
                    0x10
                } else {
                    0x90
                }
            }
            0x028 => self.ibrd,
            0x02c => self.lcr_h,
            0x030 => self.cr,
            0x034 => self.ifls,
            0x038 => self.raw_interrupt_status(),
            0x03c => self.raw_interrupt_status() & self.imsc,
            0x044 => self.imsc,
            0xfe0 => 0x11,
            0xfe4 => 0x10,
            0xfe8 => 0x14,
            0xfec => 0,
            0xff0 => 0x0d,
            0xff4 => 0xf0,
            0xff8 => 0x05,
            0xffc => 0xb1,
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: usize, value: u32) -> Option<u32> {
        match offset {
            0x000 => kor::console::putc(value as u8),
            0x004 | 0x024 | 0x048 => {}
            0x028 => self.ibrd = value,
            0x02c => self.lcr_h = value,
            0x030 => self.cr = value,
            0x034 => self.ifls = value,
            0x040 => {
                if value & 0x2 != 0 {
                    self.txris = false;
                }
            }
            0x044 => {
                self.imsc = value | 0x1;
                if value & 0x2 != 0 {
                    self.txris = true;
                }
            }
            _ => {}
        }
        (self.txris || self.has_rx()).then_some(UART_RX_IRQ)
    }

    fn has_rx(&self) -> bool {
        self.rxlen > 0
    }

    fn raw_interrupt_status(&self) -> u32 {
        u32::from(self.has_rx()) | (u32::from(self.txris) << 1)
    }
}
