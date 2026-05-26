use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::x86_64::port;

const PS2_DATA: u16 = 0x60;
const PS2_STATUS: u16 = 0x64;

static SHIFT: AtomicBool = AtomicBool::new(false);

pub fn init() {
    flush_buffer();
}

fn flush_buffer() {
    for _ in 0..32 {
        unsafe {
            if port::inb(PS2_STATUS) & 1 == 0 {
                break;
            }
            let _ = port::inb(PS2_DATA);
        }
    }
    SHIFT.store(false, Ordering::Relaxed);
}

pub fn handle_scancode(sc: u8) -> Option<u8> {
    scancode_to_char(sc)
}

pub fn poll_char() -> Option<u8> {
    let sc = poll_scancode()?;
    scancode_to_char(sc)
}

fn poll_scancode() -> Option<u8> {
    unsafe {
        if port::inb(PS2_STATUS) & 1 == 0 {
            return None;
        }
        Some(port::inb(PS2_DATA))
    }
}

fn scancode_to_char(sc: u8) -> Option<u8> {
    match sc {
        0x2A | 0x36 => {
            SHIFT.store(true, Ordering::Relaxed);
            None
        }
        0xAA | 0xB6 => {
            SHIFT.store(false, Ordering::Relaxed);
            None
        }
        _ if sc & 0x80 != 0 => None,
        0x1C => Some(b'\n'),
        0x0E => Some(0x08), // backspace sentinel
        0x39 => Some(b' '),
        code => {
            let shift = SHIFT.load(Ordering::Relaxed);
            map_key(code, shift)
        }
    }
}

fn map_key(sc: u8, shift: bool) -> Option<u8> {
    let pair = match sc {
        0x02 => (b'1', b'!'),
        0x03 => (b'2', b'@'),
        0x04 => (b'3', b'#'),
        0x05 => (b'4', b'$'),
        0x06 => (b'5', b'%'),
        0x07 => (b'6', b'^'),
        0x08 => (b'7', b'&'),
        0x09 => (b'8', b'*'),
        0x0A => (b'9', b'('),
        0x0B => (b'0', b')'),
        0x0C => (b'-', b'_'),
        0x0D => (b'=', b'+'),
        0x10 => (b'q', b'Q'),
        0x11 => (b'w', b'W'),
        0x12 => (b'e', b'E'),
        0x13 => (b'r', b'R'),
        0x14 => (b't', b'T'),
        0x15 => (b'y', b'Y'),
        0x16 => (b'u', b'U'),
        0x17 => (b'i', b'I'),
        0x18 => (b'o', b'O'),
        0x19 => (b'p', b'P'),
        0x1A => (b'[', b'{'),
        0x1B => (b']', b'}'),
        0x1E => (b'a', b'A'),
        0x1F => (b's', b'S'),
        0x20 => (b'd', b'D'),
        0x21 => (b'f', b'F'),
        0x22 => (b'g', b'G'),
        0x23 => (b'h', b'H'),
        0x24 => (b'j', b'J'),
        0x25 => (b'k', b'K'),
        0x26 => (b'l', b'L'),
        0x27 => (b';', b':'),
        0x28 => (b'\'', b'"'),
        0x29 => (b'`', b'~'),
        0x2B => (b'\\', b'|'),
        0x2C => (b'z', b'Z'),
        0x2D => (b'x', b'X'),
        0x2E => (b'c', b'C'),
        0x2F => (b'v', b'V'),
        0x30 => (b'b', b'B'),
        0x31 => (b'n', b'N'),
        0x32 => (b'm', b'M'),
        0x33 => (b',', b'<'),
        0x34 => (b'.', b'>'),
        0x35 => (b'/', b'?'),
        _ => return None,
    };
    Some(if shift { pair.1 } else { pair.0 })
}
