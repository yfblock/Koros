//! Host command shell and guest input routing for the shared physical UART.

const HOST_CONSOLE: usize = 2;
const CTRL_O: u8 = 0x0f;
const COMMAND_BYTES: usize = 64;

pub enum InputAction {
    None,
    Attach(usize),
    Status,
    GuestByte { guest: usize, byte: u8 },
}

pub struct ConsoleMux {
    active: usize,
    command: [u8; COMMAND_BYTES],
    length: usize,
}

impl ConsoleMux {
    pub const fn new() -> Self {
        Self {
            active: HOST_CONSOLE,
            command: [0; COMMAND_BYTES],
            length: 0,
        }
    }

    pub fn print_banner(&self) {
        kor::println!("Korhv console: enter 0 | enter 1 | status | help");
        kor::println!("Ctrl-O returns from a guest to the Korhv console");
        self.prompt();
    }

    pub fn input(&mut self, byte: u8) -> InputAction {
        if self.active < HOST_CONSOLE {
            if byte == CTRL_O {
                let guest = self.active;
                self.active = HOST_CONSOLE;
                kor::println!("\n[returned from guest {}]", guest);
                self.prompt();
                return InputAction::None;
            }
            return InputAction::GuestByte {
                guest: self.active,
                byte,
            };
        }
        match byte {
            b'\r' | b'\n' => self.execute_command(),
            0x08 | 0x7f if self.length > 0 => {
                self.length -= 1;
                kor::print!("\x08 \x08");
                InputAction::None
            }
            0x20..=0x7e if self.length < self.command.len() => {
                self.command[self.length] = byte;
                self.length += 1;
                kor::console::putc(byte);
                InputAction::None
            }
            _ => InputAction::None,
        }
    }

    pub fn attach(&mut self, guest: usize) {
        self.active = guest;
        kor::println!("[attached guest {}; Ctrl-O returns to Korhv]", guest);
    }

    fn execute_command(&mut self) -> InputAction {
        kor::println!();
        let command = core::str::from_utf8(&self.command[..self.length])
            .unwrap_or("")
            .trim();
        let action = match command {
            "enter 0" => InputAction::Attach(0),
            "enter 1" => InputAction::Attach(1),
            "status" => InputAction::Status,
            "help" | "" => {
                kor::println!("enter 0 | enter 1 | status | help");
                kor::println!("Ctrl-O returns from a guest to the Korhv console");
                self.prompt();
                InputAction::None
            }
            _ => {
                kor::println!("unknown command: {}", command);
                self.prompt();
                InputAction::None
            }
        };
        self.length = 0;
        action
    }

    pub fn prompt(&self) {
        kor::print!("korhv> ");
    }
}
