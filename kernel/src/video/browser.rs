//! In-kernel web browser — async HTTP fetch + text display.

use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

use super::draw::{self, rgb};
use super::framebuffer::Framebuffer;

const URL_CAP: usize = 96;
const MAX_LINES: usize = 32;
const DHCP_BUDGET: u32 = 150;
#[derive(Clone, Copy, PartialEq, Eq)]
enum LoadPhase {
    Idle,
    ProbeNic,
    WaitDhcp,
    FetchHttp,
}

struct Browser {
    url: [u8; URL_CAP],
    url_len: usize,
    pending_url: [u8; URL_CAP],
    pending_len: usize,
    load_phase: LoadPhase,
    dhcp_iters: u32,
    http_iters: u32,
    lines: Vec<String>,
    status: &'static str,
    dirty: bool,
}

impl Browser {
    const fn new() -> Self {
        Self {
            url: [0; URL_CAP],
            url_len: 0,
            pending_url: [0; URL_CAP],
            pending_len: 0,
            load_phase: LoadPhase::Idle,
            dhcp_iters: 0,
            http_iters: 0,
            lines: Vec::new(),
            status: "Type URL, press Enter (http://example.com)",
            dirty: true,
        }
    }

    fn set_default_url(&mut self) {
        if self.url_len == 0 {
            let default = b"http://example.com";
            self.url[..default.len()].copy_from_slice(default);
            self.url_len = default.len();
        }
    }

    fn url_str(&self) -> &str {
        core::str::from_utf8(&self.url[..self.url_len]).unwrap_or("")
    }
}

static BROWSER: Mutex<Browser> = Mutex::new(Browser::new());

pub fn init() {
    let mut b = BROWSER.lock();
    b.set_default_url();
    b.dirty = true;
}

pub fn mark_dirty() {
    BROWSER.lock().dirty = true;
}

pub fn is_loading() -> bool {
    BROWSER.lock().load_phase != LoadPhase::Idle
}

pub fn take_dirty() -> bool {
    let mut b = BROWSER.lock();
    let d = b.dirty;
    b.dirty = false;
    d
}

pub fn handle_key(key: u8) {
    let mut b = BROWSER.lock();
    b.set_default_url();
    match key {
        b'\n' => {
            if b.load_phase != LoadPhase::Idle {
                return;
            }
            b.set_default_url();
            let len = b.url_len;
            let mut tmp = [0u8; URL_CAP];
            tmp[..len].copy_from_slice(&b.url[..len]);
            b.pending_url[..len].copy_from_slice(&tmp[..len]);
            b.pending_len = len;
            b.load_phase = LoadPhase::ProbeNic;
            b.dhcp_iters = 0;
            b.http_iters = 0;
            b.status = "Loading...";
            b.dirty = true;
        }
        0x08 => {
            if b.url_len > 0 && b.load_phase == LoadPhase::Idle {
                b.url_len -= 1;
                b.dirty = true;
            }
        }
        c @ 0x20..=0x7E => {
            if b.load_phase == LoadPhase::Idle && b.url_len + 1 < URL_CAP {
                let i = b.url_len;
                b.url[i] = c;
                b.url_len += 1;
                b.dirty = true;
            }
        }
        _ => {}
    }
}

/// Drive background load without blocking the idle thread for long stretches.
pub fn tick() {
    let phase = BROWSER.lock().load_phase;
    if phase == LoadPhase::Idle {
        return;
    }

    match phase {
        LoadPhase::ProbeNic => tick_probe_nic(),
        LoadPhase::WaitDhcp => tick_wait_dhcp(),
        LoadPhase::FetchHttp => tick_fetch_http(),
        LoadPhase::Idle => {}
    }
}

fn tick_probe_nic() {
    if !crate::net::device::has_device() && !crate::net::drivers::probe() {
        finish_error("No network adapter found.");
        return;
    }
    crate::net::dhcp::start();
    let mut b = BROWSER.lock();
    b.load_phase = LoadPhase::WaitDhcp;
    b.dhcp_iters = 0;
    b.dirty = true;
}

fn tick_wait_dhcp() {
    if crate::net::dhcp::leased_ip().is_some() {
        let mut b = BROWSER.lock();
        b.load_phase = LoadPhase::FetchHttp;
        b.http_iters = 0;
        b.dirty = true;
        return;
    }

    for _ in 0..DHCP_BUDGET {
        crate::net::rx_poll();
        crate::net::dhcp::tick();
    }

    let mut b = BROWSER.lock();
    b.dhcp_iters += DHCP_BUDGET;
    b.dirty = true;

    if crate::net::dhcp::leased_ip().is_some() {
        b.load_phase = LoadPhase::FetchHttp;
        b.http_iters = 0;
        return;
    }

    if b.dhcp_iters >= 30_000 {
        drop(b);
        finish_error("DHCP failed. Check QEMU -netdev user.");
    }
}

fn tick_fetch_http() {
    let url = {
        let b = BROWSER.lock();
        let mut buf = [0u8; URL_CAP];
        let len = b.pending_len;
        buf[..len].copy_from_slice(&b.pending_url[..len]);
        String::from(core::str::from_utf8(&buf[..len]).unwrap_or(""))
    };

    match crate::net::http::fetch_step(&url) {
        crate::net::http::FetchStep::Done(lines) => {
            let mut b = BROWSER.lock();
            b.lines = lines;
            b.status = "Done — edit URL and press Enter";
            b.load_phase = LoadPhase::Idle;
            b.dirty = true;
            crate::video::apps::mark_dirty();
        }
        crate::net::http::FetchStep::Pending => {
            let mut b = BROWSER.lock();
            b.http_iters += 1;
            b.dirty = true;
            if b.http_iters > 200_000 {
                drop(b);
                finish_error("Request timed out.");
            }
        }
        crate::net::http::FetchStep::Error(msg) => finish_error(msg),
    }
}

fn finish_error(msg: &'static str) {
    let mut b = BROWSER.lock();
    b.lines = alloc::vec![String::from(msg)];
    b.status = "Error";
    b.load_phase = LoadPhase::Idle;
    b.dirty = true;
    crate::video::apps::mark_dirty();
}

pub fn draw(fb: &Framebuffer, x: u32, y: u32, w: u32, h: u32) {
    let b = BROWSER.lock();
    let bar_y = y + 36;
    draw::fill_rect(fb, x + 8, bar_y, w - 16, 28, rgb(32, 32, 40));
    draw::draw_rect_outline(fb, x + 8, bar_y, w - 16, 28, rgb(80, 80, 100));
    draw::draw_text(fb, x + 16, bar_y + 6, b.url_str(), rgb(240, 240, 248), None);

    let body_y = bar_y + 36;
    let body_h = h.saturating_sub(body_y - y + 8);
    draw::fill_rect(fb, x + 8, body_y, w - 16, body_h, rgb(248, 248, 252));

    let mut line_y = body_y + 8;
    draw::draw_text(fb, x + 16, line_y, b.status, rgb(80, 120, 80), None);
    line_y += 20;

    for text in b.lines.iter().take(MAX_LINES) {
        if line_y + 16 > body_y + body_h {
            break;
        }
        draw::draw_text(fb, x + 16, line_y, text, rgb(32, 32, 48), None);
        line_y += 16;
    }
}
