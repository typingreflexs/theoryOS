//! PS/2 keyboard and mouse — polled from the idle loop (no IRQ backend yet).
//!
//! Scancodes are translated to ASCII in `ps2.rs`; mouse packets in `mouse.rs`.

pub mod mouse;
pub mod ps2;

use crate::arch::x86_64::port;

const PS2_STATUS: u16 = 0x64;
const PS2_DATA: u16 = 0x60;

pub fn init() {
    ps2::init();
    mouse::init();
}

pub fn poll_devices<F>(mut on_key: F)
where
    F: FnMut(u8),
{
    loop {
        let status = unsafe { port::inb(PS2_STATUS) };
        if status & 1 == 0 {
            break;
        }
        let data = unsafe { port::inb(PS2_DATA) };
        if status & 0x20 != 0 {
            mouse::feed(data);
        } else if let Some(ch) = ps2::handle_scancode(data) {
            on_key(ch);
        }
    }
}

pub fn poll_key() -> Option<u8> {
    ps2::poll_char()
}
