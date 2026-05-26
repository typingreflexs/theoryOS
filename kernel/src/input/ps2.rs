//! PS/2 keyboard scancode → key code translator.
//!
//! Supports:
//! - Modifiers: Shift, Ctrl, Alt, CapsLock, NumLock
//! - Extended (0xE0-prefixed) keys: arrows, Home/End, PageUp/Down, Insert/Delete,
//!   right Ctrl/Alt, keypad slash and Enter
//! - F1–F12
//!
//! Printable keys are returned as ASCII bytes. Non-printable keys are returned as
//! key codes ≥ `KEY_FIRST` (0x80). Modifier press/release events update internal
//! state and never produce a key event.

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use crate::arch::x86_64::port;

const PS2_DATA: u16 = 0x60;
const PS2_STATUS: u16 = 0x64;

pub const KEY_FIRST: u8 = 0x80;
pub const KEY_UP: u8 = 0x80;
pub const KEY_DOWN: u8 = 0x81;
pub const KEY_LEFT: u8 = 0x82;
pub const KEY_RIGHT: u8 = 0x83;
pub const KEY_HOME: u8 = 0x84;
pub const KEY_END: u8 = 0x85;
pub const KEY_PAGE_UP: u8 = 0x86;
pub const KEY_PAGE_DOWN: u8 = 0x87;
pub const KEY_INSERT: u8 = 0x88;
pub const KEY_DELETE: u8 = 0x89;
pub const KEY_F1: u8 = 0x8A;
pub const KEY_F12: u8 = 0x95;

static SHIFT: AtomicBool = AtomicBool::new(false);
static CTRL: AtomicBool = AtomicBool::new(false);
static ALT: AtomicBool = AtomicBool::new(false);
static CAPS_LOCK: AtomicBool = AtomicBool::new(false);
static EXT_PREFIX: AtomicBool = AtomicBool::new(false);
static PAUSE_REMAINING: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy, Debug, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub caps_lock: bool,
}

pub fn modifiers() -> Modifiers {
    Modifiers {
        shift: SHIFT.load(Ordering::Relaxed),
        ctrl: CTRL.load(Ordering::Relaxed),
        alt: ALT.load(Ordering::Relaxed),
        caps_lock: CAPS_LOCK.load(Ordering::Relaxed),
    }
}

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
    CTRL.store(false, Ordering::Relaxed);
    ALT.store(false, Ordering::Relaxed);
    EXT_PREFIX.store(false, Ordering::Relaxed);
}

pub fn handle_scancode(sc: u8) -> Option<u8> {
    if PAUSE_REMAINING.load(Ordering::Relaxed) > 0 {
        let n = PAUSE_REMAINING.fetch_sub(1, Ordering::Relaxed);
        let _ = n;
        return None;
    }

    if sc == 0xE0 {
        EXT_PREFIX.store(true, Ordering::Relaxed);
        return None;
    }
    if sc == 0xE1 {
        // Pause/Break — 8-byte sequence; swallow the next 5 bytes (we already consumed E1).
        PAUSE_REMAINING.store(5, Ordering::Relaxed);
        return None;
    }

    let extended = EXT_PREFIX.swap(false, Ordering::Relaxed);
    let released = sc & 0x80 != 0;
    let code = sc & 0x7F;

    if extended {
        return handle_extended(code, released);
    }

    match code {
        0x2A | 0x36 => {
            SHIFT.store(!released, Ordering::Relaxed);
            None
        }
        0x1D => {
            CTRL.store(!released, Ordering::Relaxed);
            None
        }
        0x38 => {
            ALT.store(!released, Ordering::Relaxed);
            None
        }
        0x3A if !released => {
            CAPS_LOCK.store(!CAPS_LOCK.load(Ordering::Relaxed), Ordering::Relaxed);
            None
        }
        _ if released => None,
        0x1C => Some(b'\n'),
        0x0E => Some(0x08),
        0x0F => Some(b'\t'),
        0x01 => Some(0x1B),
        0x39 => Some(b' '),
        0x3B..=0x44 => Some(KEY_F1 + (code - 0x3B)),
        0x57 => Some(KEY_F1 + 10), // F11
        0x58 => Some(KEY_F1 + 11), // F12
        c => map_main_key(c),
    }
}

fn handle_extended(code: u8, released: bool) -> Option<u8> {
    match code {
        0x1D => {
            CTRL.store(!released, Ordering::Relaxed);
            None
        }
        0x38 => {
            ALT.store(!released, Ordering::Relaxed);
            None
        }
        _ if released => None,
        0x48 => Some(KEY_UP),
        0x50 => Some(KEY_DOWN),
        0x4B => Some(KEY_LEFT),
        0x4D => Some(KEY_RIGHT),
        0x47 => Some(KEY_HOME),
        0x4F => Some(KEY_END),
        0x49 => Some(KEY_PAGE_UP),
        0x51 => Some(KEY_PAGE_DOWN),
        0x52 => Some(KEY_INSERT),
        0x53 => Some(KEY_DELETE),
        0x1C => Some(b'\n'), // keypad enter
        0x35 => Some(b'/'),  // keypad slash
        _ => None,
    }
}

pub fn poll_char() -> Option<u8> {
    let sc = poll_scancode()?;
    handle_scancode(sc)
}

fn poll_scancode() -> Option<u8> {
    unsafe {
        if port::inb(PS2_STATUS) & 1 == 0 {
            return None;
        }
        Some(port::inb(PS2_DATA))
    }
}

fn map_main_key(sc: u8) -> Option<u8> {
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
    let shift = SHIFT.load(Ordering::Relaxed);
    let caps = CAPS_LOCK.load(Ordering::Relaxed);
    let is_letter = pair.0.is_ascii_lowercase();
    let upper = if is_letter { shift ^ caps } else { shift };
    Some(if upper { pair.1 } else { pair.0 })
}
