#![no_std]

pub mod capability;
pub mod gdt;
pub mod idt;
pub mod ipc;
pub mod memory;
pub mod paging;
pub mod pci;
pub mod pic;
pub mod syscall;
pub mod task;

pub struct SerialPort(pub u16);

impl core::fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.write_str(s);
        Ok(())
    }
}

impl SerialPort {
    pub fn new(port: u16) -> Self {
        unsafe {
            core::arch::asm!(
                "out dx, al",
                in("dx") port + 1,
                in("al") 0x00u8,
            );
            core::arch::asm!(
                "out dx, al",
                in("dx") port + 3,
                in("al") 0x80u8,
            );
            core::arch::asm!(
                "out dx, al",
                in("dx") port,
                in("al") 0x03u8,
            );
            core::arch::asm!(
                "out dx, al",
                in("dx") port + 1,
                in("al") 0x00u8,
            );
            core::arch::asm!(
                "out dx, al",
                in("dx") port + 3,
                in("al") 0x03u8,
            );
        }
        Self(port)
    }

    pub fn write_byte(&mut self, byte: u8) {
        unsafe {
            let mut line_status: u8;
            loop {
                core::arch::asm!(
                    "in al, dx",
                    out("al") line_status,
                    in("dx") self.0 + 5,
                );
                if line_status & 0x20 != 0 {
                    break;
                }
            }
            core::arch::asm!(
                "out dx, al",
                in("dx") self.0,
                in("al") byte,
            );
        }
    }

    pub fn write_str(&mut self, s: &str) {
        for b in s.bytes() {
            self.write_byte(b);
        }
    }

    pub fn write_hex(&mut self, mut val: u64) {
        self.write_str("0x");
        if val == 0 {
            self.write_byte(b'0');
            return;
        }
        let mut buf = [0u8; 16];
        let mut i = 15;
        while val > 0 {
            let digit = (val & 0xF) as u8;
            buf[i] = if digit < 10 {
                b'0' + digit
            } else {
                b'A' + (digit - 10)
            };
            val >>= 4;
            if i == 0 {
                break;
            }
            i -= 1;
        }
        for &b in &buf[i..=15] {
            if b != 0 {
                self.write_byte(b);
            }
        }
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        {
            use core::fmt::Write;
            let mut serial = $crate::SerialPort(0x3F8);
            let _ = write!(serial, $($arg)*);
        }
    };
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", core::format_args!($($arg)*)));
}
