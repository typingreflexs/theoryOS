use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use spin::Mutex;

use crate::arch::x86_64::port;

const PS2_STATUS: u16 = 0x64;
const PS2_DATA: u16 = 0x60;

static FB_W: AtomicU32 = AtomicU32::new(1024);
static FB_H: AtomicU32 = AtomicU32::new(768);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseState {
    pub x: i32,
    pub y: i32,
    pub buttons: u8,
    pub present: bool,
}

static STATE: Mutex<Inner> = Mutex::new(Inner::new());
static MOVED: AtomicBool = AtomicBool::new(false);
static CLICKED: AtomicBool = AtomicBool::new(false);

struct Inner {
    x: i32,
    y: i32,
    buttons: u8,
    packet: [u8; 3],
    packet_idx: u8,
    present: bool,
}

impl Inner {
    const fn new() -> Self {
        Self {
            x: 400,
            y: 300,
            buttons: 0,
            packet: [0; 3],
            packet_idx: 0,
            present: false,
        }
    }
}

pub fn init() {
    ps2_wait();
    unsafe { port::outb(PS2_STATUS, 0xA8) }; // enable aux port
    if mouse_cmd(0xF6).is_ok() && mouse_cmd(0xF4).is_ok() {
        STATE.lock().present = true;
        crate::console::Console::println("[input] PS/2 mouse online");
    }
}

fn mouse_cmd(val: u8) -> Result<(), ()> {
    ps2_wait();
    unsafe { port::outb(PS2_STATUS, 0xD4) };
    ps2_wait();
    unsafe { port::outb(PS2_DATA, val) };
    Ok(())
}

fn ps2_wait() {
    for _ in 0..100_000 {
        unsafe {
            if port::inb(PS2_STATUS) & 2 == 0 {
                return;
            }
        }
        crate::arch::x86_64::cpu::pause();
    }
}

pub fn set_bounds(width: u32, height: u32) {
    FB_W.store(width, Ordering::Relaxed);
    FB_H.store(height, Ordering::Relaxed);
    let mut s = STATE.lock();
    s.x = s.x.clamp(0, width.saturating_sub(1) as i32);
    s.y = s.y.clamp(0, height.saturating_sub(1) as i32);
}

pub fn feed(byte: u8) {
    let mut s = STATE.lock();
    if s.packet_idx == 0 && byte & 0x08 == 0 {
        return;
    }
    let idx = s.packet_idx as usize;
    s.packet[idx] = byte;
    s.packet_idx += 1;
    if s.packet_idx < 3 {
        return;
    }
    s.packet_idx = 0;
    s.present = true;

    let dx = s.packet[1] as i8 as i32;
    let dy = -(s.packet[2] as i8 as i32);
    s.x += dx;
    s.y += dy;
    let max_x = FB_W.load(Ordering::Relaxed).saturating_sub(1) as i32;
    let max_y = FB_H.load(Ordering::Relaxed).saturating_sub(1) as i32;
    s.x = s.x.clamp(0, max_x);
    s.y = s.y.clamp(0, max_y);
    let prev = s.buttons;
    s.buttons = s.packet[0] & 0x07;
    MOVED.store(true, Ordering::Release);
    let left_now = s.buttons & 1 != 0;
    let left_prev = prev & 1 != 0;
    if left_now && !left_prev {
        CLICKED.store(true, Ordering::Release);
    }
}

pub fn state() -> MouseState {
    let s = STATE.lock();
    MouseState {
        x: s.x,
        y: s.y,
        buttons: s.buttons,
        present: s.present,
    }
}

pub fn take_moved() -> bool {
    MOVED.swap(false, Ordering::AcqRel)
}

pub fn take_clicked() -> bool {
    CLICKED.swap(false, Ordering::AcqRel)
}

pub fn left_down() -> bool {
    STATE.lock().buttons & 1 != 0
}
