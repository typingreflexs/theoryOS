//! In-kernel web browser application.
//!
//! Asynchronous: clicking Enter (or the Go button) submits a URL which is
//! resolved → connected → fetched on the idle thread via incremental
//! `http::fetch_step` calls. While loading, the URL bar stays interactive and
//! the user can hit Tab to switch between URL-edit mode and page-scroll mode.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

use super::draw::{self, rgb, text_width};
use super::framebuffer::Framebuffer;
use crate::input::ps2;
use crate::net::http::{self, FetchResult, FetchStep, LineStyle, RenderedLine};

const URL_CAP: usize = 128;
const DHCP_BUDGET: u32 = 100;
const HISTORY_CAP: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LoadPhase {
    Idle,
    ProbeNic,
    WaitDhcp,
    FetchHttp,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Focus {
    UrlBar,
    Page,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hotspot {
    Go,
    Stop,
    Back,
    ScrollUp,
    ScrollDown,
    Home,
}

struct Browser {
    url: [u8; URL_CAP],
    url_len: usize,
    pending_url: String,
    load_phase: LoadPhase,
    dhcp_started_ns: u64,
    fetch_started_ns: u64,
    focus: Focus,
    lines: Vec<RenderedLine>,
    history: Vec<String>,
    status: String,
    last_result: Option<FetchResult>,
    scroll: usize,
    visible_lines: usize,
    last_error: Option<String>,
    dirty: bool,
    hotspots: Vec<(Hotspot, u32, u32, u32, u32)>,
}

impl Browser {
    const fn new() -> Self {
        Self {
            url: [0; URL_CAP],
            url_len: 0,
            pending_url: String::new(),
            load_phase: LoadPhase::Idle,
            dhcp_started_ns: 0,
            fetch_started_ns: 0,
            focus: Focus::UrlBar,
            lines: Vec::new(),
            history: Vec::new(),
            status: String::new(),
            last_result: None,
            scroll: 0,
            visible_lines: 16,
            last_error: None,
            dirty: true,
            hotspots: Vec::new(),
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

    fn set_url(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let n = bytes.len().min(URL_CAP);
        self.url[..n].copy_from_slice(&bytes[..n]);
        self.url_len = n;
    }
}

static BROWSER: Mutex<Browser> = Mutex::new(Browser::new());

pub fn init() {
    let mut b = BROWSER.lock();
    b.set_default_url();
    b.status = String::from("Ready. Edit URL and press Enter or Go.");
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
    if b.url_len == 0 && b.focus == Focus::UrlBar {
        b.set_default_url();
    }
    match key {
        b'\t' => {
            b.focus = match b.focus {
                Focus::UrlBar => Focus::Page,
                Focus::Page => Focus::UrlBar,
            };
            b.dirty = true;
        }
        b'\n' => {
            if b.focus == Focus::UrlBar {
                submit(&mut b);
            } else {
                // Enter in page mode reloads.
                let url = String::from(b.url_str());
                if !url.is_empty() {
                    b.pending_url = url;
                    start_load(&mut b);
                }
            }
        }
        0x08 => {
            if b.focus == Focus::UrlBar && b.url_len > 0 {
                b.url_len -= 1;
                b.dirty = true;
            }
        }
        0x1B => {
            if b.load_phase != LoadPhase::Idle {
                cancel_load(&mut b, "Cancelled by user");
            }
        }
        ps2::KEY_PAGE_DOWN => {
            let v = b.visible_lines as isize;
            scroll_by(&mut b, v);
        }
        ps2::KEY_PAGE_UP => {
            let v = b.visible_lines as isize;
            scroll_by(&mut b, -v);
        }
        ps2::KEY_DOWN => {
            scroll_by(&mut b, 1);
        }
        ps2::KEY_UP => {
            scroll_by(&mut b, -1);
        }
        ps2::KEY_HOME => {
            let n = b.lines.len() as isize;
            scroll_by(&mut b, -n);
        }
        ps2::KEY_END => {
            let n = b.lines.len() as isize;
            scroll_by(&mut b, n);
        }
        b' ' if b.focus == Focus::Page => {
            let v = b.visible_lines as isize;
            scroll_by(&mut b, v);
        }
        b'b' if b.focus == Focus::Page => {
            let v = b.visible_lines as isize;
            scroll_by(&mut b, -v);
        }
        c @ 0x20..=0x7E => {
            if b.focus == Focus::UrlBar && b.url_len + 1 < URL_CAP {
                let i = b.url_len;
                b.url[i] = c;
                b.url_len += 1;
                b.dirty = true;
            }
        }
        _ => {}
    }
}

fn submit(b: &mut Browser) {
    if b.load_phase != LoadPhase::Idle {
        return;
    }
    let url = String::from(b.url_str());
    if url.is_empty() {
        return;
    }
    push_history(b, &url);
    b.pending_url = url;
    start_load(b);
}

fn push_history(b: &mut Browser, url: &str) {
    if b.history.last().map(|s| s.as_str()) == Some(url) {
        return;
    }
    b.history.push(String::from(url));
    if b.history.len() > HISTORY_CAP {
        b.history.remove(0);
    }
}

fn start_load(b: &mut Browser) {
    b.load_phase = LoadPhase::ProbeNic;
    b.fetch_started_ns = crate::sched::timer::monotonic_ns();
    b.status = String::from("Initializing network adapter...");
    b.last_error = None;
    b.scroll = 0;
    b.dirty = true;
    http::fetch_cancel();
}

fn cancel_load(b: &mut Browser, reason: &str) {
    http::fetch_cancel();
    b.load_phase = LoadPhase::Idle;
    b.status = String::from(reason);
    b.last_error = Some(String::from(reason));
    b.dirty = true;
}

fn scroll_by(b: &mut Browser, delta: isize) {
    let max = b.lines.len().saturating_sub(b.visible_lines);
    let cur = b.scroll as isize + delta;
    b.scroll = cur.clamp(0, max as isize) as usize;
    b.dirty = true;
}

fn back(b: &mut Browser) {
    if b.history.len() > 1 {
        b.history.pop();
    }
    if let Some(prev) = b.history.last().cloned() {
        b.set_url(&prev);
        b.pending_url = prev;
        start_load(b);
    }
}

fn go_home(b: &mut Browser) {
    let home = "http://example.com";
    b.set_url(home);
    push_history(b, home);
    b.pending_url = String::from(home);
    start_load(b);
}

/// Mouse click coordinates relative to the framebuffer.
pub fn handle_click(x: u32, y: u32) -> bool {
    let hotspots = BROWSER.lock().hotspots.clone();
    for (kind, hx, hy, hw, hh) in hotspots {
        if x >= hx && x < hx + hw && y >= hy && y < hy + hh {
            let mut b = BROWSER.lock();
            match kind {
                Hotspot::Go => submit(&mut b),
                Hotspot::Stop => cancel_load(&mut b, "Stopped"),
                Hotspot::Back => back(&mut b),
                Hotspot::Home => go_home(&mut b),
                Hotspot::ScrollUp => {
                    let v = b.visible_lines as isize;
                    scroll_by(&mut b, -v);
                }
                Hotspot::ScrollDown => {
                    let v = b.visible_lines as isize;
                    scroll_by(&mut b, v);
                }
            }
            return true;
        }
    }
    false
}

/// Drive the asynchronous load. Called from the idle loop.
pub fn tick() {
    let phase = BROWSER.lock().load_phase;
    match phase {
        LoadPhase::Idle => {}
        LoadPhase::ProbeNic => tick_probe_nic(),
        LoadPhase::WaitDhcp => tick_wait_dhcp(),
        LoadPhase::FetchHttp => tick_fetch_http(),
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
    b.dhcp_started_ns = crate::sched::timer::monotonic_ns();
    b.status = String::from("Requesting IP address (DHCP)...");
    b.dirty = true;
}

fn tick_wait_dhcp() {
    let now = crate::sched::timer::monotonic_ns();
    if crate::net::dhcp::leased_ip().is_some() {
        let mut b = BROWSER.lock();
        b.load_phase = LoadPhase::FetchHttp;
        b.status = String::from("Connecting...");
        b.dirty = true;
        return;
    }

    for _ in 0..DHCP_BUDGET {
        crate::net::rx_poll();
        crate::net::dhcp::tick();
        if crate::net::dhcp::leased_ip().is_some() {
            break;
        }
    }

    let started = BROWSER.lock().dhcp_started_ns;
    if now.saturating_sub(started) > 15_000_000_000 {
        finish_error("DHCP timed out. Check QEMU -netdev user.");
        return;
    }
    BROWSER.lock().dirty = true;
}

fn tick_fetch_http() {
    let url = BROWSER.lock().pending_url.clone();
    match crate::net::http::fetch_step(&url) {
        FetchStep::Done(result) => {
            let mut b = BROWSER.lock();
            b.last_result = Some(result.clone());
            b.lines = result.lines;
            b.status = format!(
                "{} — {} ({} bytes)",
                result.host,
                result.status_code,
                result.bytes_received
            );
            b.load_phase = LoadPhase::Idle;
            b.scroll = 0;
            b.focus = Focus::Page;
            b.dirty = true;
            crate::video::apps::mark_dirty();
        }
        FetchStep::Pending => {
            let mut b = BROWSER.lock();
            if let Some(p) = http::current_progress() {
                if let Some(ip) = p.remote_ip {
                    b.status = format!(
                        "{} {} ({}.{}.{}.{}) — {} bytes",
                        p.phase.label(),
                        p.host,
                        ip.0[0],
                        ip.0[1],
                        ip.0[2],
                        ip.0[3],
                        p.bytes_received
                    );
                } else {
                    b.status =
                        format!("{} {} — {} bytes", p.phase.label(), p.host, p.bytes_received);
                }
            }
            b.dirty = true;
        }
        FetchStep::Error(msg) => finish_error(msg),
    }
}

fn finish_error(msg: &'static str) {
    let mut b = BROWSER.lock();
    b.lines = alloc::vec![RenderedLine {
        text: String::from(msg),
        style: LineStyle::Body,
    }];
    b.status = String::from(msg);
    b.last_error = Some(String::from(msg));
    b.load_phase = LoadPhase::Idle;
    b.dirty = true;
    crate::video::apps::mark_dirty();
}

pub fn draw(fb: &Framebuffer, x: u32, y: u32, w: u32, h: u32) {
    let mut b = BROWSER.lock();
    b.hotspots.clear();

    // Toolbar
    let toolbar_y = y + 36;
    draw::fill_rect(fb, x + 8, toolbar_y, w - 16, 30, rgb(40, 40, 56));
    let btn_w = 48u32;
    let btn_h = 24u32;
    let btn_y = toolbar_y + 3;
    let mut bx = x + 12;

    draw_button(fb, &mut b, bx, btn_y, btn_w, btn_h, "Back", rgb(60, 60, 90), Hotspot::Back);
    bx += btn_w + 4;
    draw_button(fb, &mut b, bx, btn_y, btn_w, btn_h, "Home", rgb(60, 80, 60), Hotspot::Home);
    bx += btn_w + 4;
    let is_loading = b.load_phase != LoadPhase::Idle;
    if is_loading {
        draw_button(fb, &mut b, bx, btn_y, btn_w, btn_h, "Stop", rgb(160, 60, 60), Hotspot::Stop);
    } else {
        draw_button(fb, &mut b, bx, btn_y, btn_w, btn_h, "Go", rgb(60, 120, 220), Hotspot::Go);
    }
    bx += btn_w + 8;

    // URL bar
    let url_x = bx;
    let url_w = (x + w - 8).saturating_sub(url_x + 12);
    let url_active = b.focus == Focus::UrlBar;
    draw::fill_rect(fb, url_x, btn_y, url_w, btn_h, rgb(20, 20, 28));
    draw::draw_rect_outline(
        fb,
        url_x,
        btn_y,
        url_w,
        btn_h,
        if url_active { rgb(120, 200, 240) } else { rgb(80, 80, 100) },
    );
    let url_text = format!(
        "{}{}",
        b.url_str(),
        if url_active { "_" } else { "" }
    );
    draw::draw_text(fb, url_x + 6, btn_y + 5, &url_text, rgb(230, 230, 240), None);

    // Status bar
    let status_y = toolbar_y + 36;
    draw::fill_rect(fb, x + 8, status_y, w - 16, 22, rgb(28, 28, 40));
    let status_color = if b.last_error.is_some() {
        rgb(220, 140, 140)
    } else if is_loading {
        rgb(220, 200, 120)
    } else {
        rgb(140, 200, 140)
    };
    draw::draw_text(fb, x + 16, status_y + 3, &b.status, status_color, None);

    // Body
    let body_y = status_y + 28;
    let body_h = h.saturating_sub(body_y - y + 8);
    draw::fill_rect(fb, x + 8, body_y, w - 16, body_h, rgb(248, 248, 252));

    // Scroll buttons on the right side
    let scrollbar_x = x + w - 30;
    draw_button(
        fb,
        &mut b,
        scrollbar_x,
        body_y + 4,
        22,
        22,
        "^",
        rgb(80, 80, 110),
        Hotspot::ScrollUp,
    );
    draw_button(
        fb,
        &mut b,
        scrollbar_x,
        body_y + body_h - 26,
        22,
        22,
        "v",
        rgb(80, 80, 110),
        Hotspot::ScrollDown,
    );

    // Render lines
    let line_h = 16u32;
    let content_x = x + 16;
    let content_w = w.saturating_sub(50);
    let max_visible = (body_h.saturating_sub(16) / line_h) as usize;
    b.visible_lines = max_visible.max(4);

    let max_scroll = b.lines.len().saturating_sub(b.visible_lines);
    if b.scroll > max_scroll {
        b.scroll = max_scroll;
    }
    let start = b.scroll;
    let end = (start + b.visible_lines).min(b.lines.len());

    let mut line_y = body_y + 8;
    for i in start..end {
        let line = &b.lines[i];
        let color = match line.style {
            LineStyle::Heading => rgb(40, 60, 140),
            LineStyle::Link => rgb(80, 80, 200),
            LineStyle::Code => rgb(120, 60, 40),
            LineStyle::Body => rgb(32, 32, 48),
        };
        let display = clip_text(&line.text, content_w as usize);
        draw::draw_text(fb, content_x, line_y, &display, color, None);
        line_y += line_h;
    }

    // Page indicator
    let page_text = if b.lines.is_empty() {
        String::from("0/0")
    } else {
        let cur = b.scroll / b.visible_lines.max(1) + 1;
        let total = (b.lines.len() + b.visible_lines - 1) / b.visible_lines.max(1);
        format!("{}/{}", cur, total)
    };
    let pw = text_width(&page_text);
    draw::draw_text(
        fb,
        x + w - pw - 36,
        body_y + body_h - 18,
        &page_text,
        rgb(120, 120, 140),
        None,
    );
}

fn draw_button(
    fb: &Framebuffer,
    b: &mut Browser,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    label: &str,
    color: u32,
    hotspot: Hotspot,
) {
    draw::fill_rect(fb, x, y, w, h, color);
    draw::draw_rect_outline(fb, x, y, w, h, rgb(220, 220, 240));
    let tw = text_width(label);
    let tx = x + (w.saturating_sub(tw)) / 2;
    let ty = y + (h.saturating_sub(16)) / 2;
    draw::draw_text(fb, tx, ty, label, rgb(245, 245, 250), None);
    b.hotspots.push((hotspot, x, y, w, h));
}

fn clip_text(s: &str, max_pixels: usize) -> String {
    let glyph_w = 8usize;
    let max_chars = max_pixels / glyph_w;
    if s.len() <= max_chars {
        return String::from(s);
    }
    let mut t: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    t.push('…');
    t
}
