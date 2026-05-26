use core::fmt::{self, Write};

const COM1: u16 = 0x3F8;

pub struct SerialPort {
    hhdm: u64,
}

impl SerialPort {
    pub fn new(hhdm: u64) -> Self {
        Self { hhdm }
    }

    pub fn init_early(_hhdm: u64) {
        // COM1 is identity-mapped by Limine in the HHDM lower canonical region.
        unsafe {
            outb(COM1 + 1, 0x00);
            outb(COM1 + 3, 0x80);
            outb(COM1 + 0, 0x03);
            outb(COM1 + 1, 0x00);
            outb(COM1 + 3, 0x03);
            outb(COM1 + 2, 0xC7);
            outb(COM1 + 4, 0x0B);
        }
    }

    fn can_transmit(&self) -> bool {
        unsafe { inb(COM1 + 5) & 0x20 != 0 }
    }

    fn write_byte(&self, byte: u8) {
        while !self.can_transmit() {}
        unsafe { outb(COM1, byte) };
    }
}

impl Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(byte);
        }
        Ok(())
    }
}

unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack));
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack));
    value
}
