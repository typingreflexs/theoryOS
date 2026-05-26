use spin::Mutex;

use super::draw::{self, rgb, text_width};
use super::desktop::{self, TASKBAR_H};
use super::framebuffer::Framebuffer;
use crate::input::mouse;
use crate::net::{self, dhcp, wifi};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppId {
    Console,
    Settings,
    Browser,
    Files,
    Wifi,
}

struct DesktopApps {
    launcher_open: bool,
    focus: AppId,
    dirty: bool,
    net_ready: bool,
}

impl DesktopApps {
    const fn new() -> Self {
        Self {
            launcher_open: false,
            focus: AppId::Console,
            dirty: true,
            net_ready: false,
        }
    }
}

static APPS: Mutex<DesktopApps> = Mutex::new(DesktopApps::new());

pub fn init() {
    wifi::init();
}

pub fn init_network_once() {
    let mut apps = APPS.lock();
    if apps.net_ready {
        return;
    }
    apps.net_ready = true;
    drop(apps);
    if !net::device::has_device() {
        net::drivers::probe();
    }
    net::dhcp::start();
}

pub fn mark_dirty() {
    APPS.lock().dirty = true;
}

pub fn take_dirty() -> bool {
    let mut apps = APPS.lock();
    let d = apps.dirty;
    apps.dirty = false;
    d
}

pub fn focus() -> AppId {
    APPS.lock().focus
}

pub fn open(app: AppId) {
    let mut apps = APPS.lock();
    apps.focus = app;
    apps.launcher_open = false;
    apps.dirty = true;
    drop(apps);
    if app == AppId::Settings || app == AppId::Wifi {
        init_network_once();
    }
}

pub fn toggle_launcher() {
    let mut apps = APPS.lock();
    apps.launcher_open = !apps.launcher_open;
    apps.dirty = true;
}

pub fn handle_click(x: i32, y: i32, fb: &Framebuffer) -> bool {
    if x < 0 || y < 0 {
        return false;
    }
    let ux = x as u32;
    let uy = y as u32;
    let ty = desktop::taskbar_y(fb);

    // Start button
    if uy >= ty + 6 && uy <= ty + TASKBAR_H - 6 && ux >= 8 && ux <= 128 {
        toggle_launcher();
        return true;
    }

    let mut apps = APPS.lock();
    if apps.launcher_open {
        if let Some(app) = launcher_hit(ux, uy, fb) {
            drop(apps);
            open(app);
            return true;
        }
        if uy < ty {
            apps.launcher_open = false;
            apps.dirty = true;
            return true;
        }
    }

    if apps.focus == AppId::Console {
        drop(apps);
        return false;
    }
    false
}

fn launcher_hit(x: u32, y: u32, fb: &Framebuffer) -> Option<AppId> {
    let wh = desktop::work_height(fb);
    let tile_w = 110u32;
    let tile_h = 90u32;
    let cols = 3u32;
    let grid_w = cols * tile_w + (cols - 1) * 16;
    let start_x = fb.width.saturating_sub(grid_w) / 2;
    let start_y = wh / 4;
    let apps = [
        AppId::Console,
        AppId::Settings,
        AppId::Browser,
        AppId::Files,
        AppId::Wifi,
    ];
    for (i, app) in apps.iter().enumerate() {
        let col = (i as u32) % cols;
        let row = (i as u32) / cols;
        let tx = start_x + col * (tile_w + 16);
        let ty = start_y + row * (tile_h + 16);
        if x >= tx && x < tx + tile_w && y >= ty && y < ty + tile_h {
            return Some(*app);
        }
    }
    None
}

pub fn draw(fb: &Framebuffer) {
    let apps = APPS.lock();
    match apps.focus {
        AppId::Console => {}
        AppId::Settings => draw_settings(fb),
        AppId::Browser => draw_browser(fb),
        AppId::Files => draw_files(fb),
        AppId::Wifi => draw_wifi(fb),
    }
    if apps.launcher_open {
        draw_launcher(fb);
    }
    drop(apps);
    draw_cursor(fb);
}

fn window_frame(fb: &Framebuffer, title: &str) -> (u32, u32, u32, u32) {
    let wh = desktop::work_height(fb);
    let w = fb.width.saturating_sub(80).min(640).max(400);
    let h = wh.saturating_sub(80).min(420).max(240);
    let x = (fb.width - w) / 2;
    let y = (wh - h) / 3;
    draw::fill_rect(fb, x, y, w, h, rgb(20, 20, 28));
    draw::fill_rect(fb, x, y, w, 32, rgb(48, 72, 140));
    draw::draw_rect_outline(fb, x, y, w, h, rgb(100, 100, 120));
    draw::draw_text(fb, x + 12, y + 8, title, rgb(240, 240, 248), None);
    (x, y, w, h)
}

fn draw_settings(fb: &Framebuffer) {
    let (x, y, w, h) = window_frame(fb, "Settings");
    let body_y = y + 44;
    let mut line = body_y;
    let lh = 20u32;

    draw_line(fb, x + 16, line, "System", rgb(120, 180, 255));
    line += lh;
    draw_line(fb, x + 24, line, "Hostname: theory-os", rgb(210, 210, 220));
    line += lh;
    draw_line(
        fb,
        x + 24,
        line,
        &alloc::format!("Display: {}x{}", fb.width, fb.height),
        rgb(210, 210, 220),
    );
    line += lh + 8;
    draw_line(fb, x + 16, line, "Network", rgb(120, 180, 255));
    line += lh;
    if let Some(ip) = dhcp::leased_ip() {
        draw_line(
            fb,
            x + 24,
            line,
            &alloc::format!(
                "Ethernet: {}.{}.{}.{} (DHCP)",
                ip.0[0],
                ip.0[1],
                ip.0[2],
                ip.0[3]
            ),
            rgb(210, 210, 220),
        );
    } else {
        draw_line(fb, x + 24, line, &wifi::ethernet_line(), rgb(160, 160, 176));
    }
    line += lh;
    draw_line(fb, x + 24, line, wifi::wifi_line(), rgb(160, 160, 176));
    line += lh + 8;
    draw_line(fb, x + 16, line, "Devices", rgb(120, 180, 255));
    line += lh;
    let mouse = mouse::state();
    if mouse.present {
        draw_line(fb, x + 24, line, "Mouse: PS/2 detected", rgb(210, 210, 220));
    } else {
        draw_line(fb, x + 24, line, "Mouse: not detected", rgb(160, 160, 176));
    }
    line += lh;
    draw_line(fb, x + 24, line, "Keyboard: PS/2", rgb(210, 210, 220));
    let _ = (w, h);
}

fn draw_browser(fb: &Framebuffer) {
    let (x, y, w, h) = window_frame(fb, "Browser");
    super::browser::draw(fb, x, y, w, h);
}

fn draw_files(fb: &Framebuffer) {
    let (x, y, w, h) = window_frame(fb, "Files");
    let body_y = y + 44;
    draw::fill_rect(fb, x + 8, body_y, w - 16, h - 52, rgb(12, 12, 16));
    let entries = ["bin", "dev", "etc", "lib", "proc", "sys", "tmp"];
    let mut line = body_y + 8;
    for entry in entries {
        draw::draw_text(fb, x + 16, line, entry, rgb(210, 210, 220), None);
        line += 18;
    }
}

fn draw_wifi(fb: &Framebuffer) {
    let (x, y, w, h) = window_frame(fb, "Network");
    let body_y = y + 44;
    draw::fill_rect(fb, x + 8, body_y, w - 16, h - 52, rgb(12, 12, 16));
    let mut line = body_y + 8;
    draw_line(fb, x + 16, line, &wifi::status_line(), rgb(120, 220, 120));
    line += 24;
    draw_line(fb, x + 16, line, &wifi::ethernet_line(), rgb(210, 210, 220));
    line += 24;
    draw_line(fb, x + 16, line, wifi::wifi_line(), rgb(160, 160, 176));
    line += 24;
    draw_line(fb, x + 16, line, "Open Browser to load web pages.", rgb(140, 140, 160));
}

fn draw_launcher(fb: &Framebuffer) {
    let wh = desktop::work_height(fb);
    draw::fill_rect(fb, 0, 0, fb.width, wh, rgb(8, 8, 16));
    draw::draw_text(fb, 24, 20, "Applications", rgb(240, 240, 248), None);

    let tiles: [(&str, AppId); 5] = [
        ("Console", AppId::Console),
        ("Settings", AppId::Settings),
        ("Browser", AppId::Browser),
        ("Files", AppId::Files),
        ("Wi-Fi", AppId::Wifi),
    ];
    let tile_w = 110u32;
    let tile_h = 90u32;
    let cols = 3u32;
    let grid_w = cols * tile_w + (cols - 1) * 16;
    let start_x = fb.width.saturating_sub(grid_w) / 2;
    let start_y = wh / 4;

    for (i, (label, _)) in tiles.iter().enumerate() {
        let col = (i as u32) % cols;
        let row = (i as u32) / cols;
        let tx = start_x + col * (tile_w + 16);
        let ty = start_y + row * (tile_h + 16);
        draw::fill_rect(fb, tx, ty, tile_w, tile_h, rgb(48, 48, 64));
        draw::draw_rect_outline(fb, tx, ty, tile_w, tile_h, rgb(96, 120, 200));
        let tw = text_width(label);
        draw::draw_text(
            fb,
            tx + (tile_w - tw) / 2,
            ty + tile_h / 2 - 8,
            label,
            rgb(240, 240, 248),
            None,
        );
    }
}

fn draw_line(fb: &Framebuffer, x: u32, y: u32, text: &str, color: u32) {
    draw::draw_text(fb, x, y, text, color, None);
}

fn draw_cursor(fb: &Framebuffer) {
    let m = mouse::state();
    if !m.present {
        return;
    }
    let x = m.x.clamp(0, fb.width.saturating_sub(1) as i32) as u32;
    let y = m.y.clamp(0, fb.height.saturating_sub(1) as i32) as u32;
    draw::fill_rect(fb, x, y, 2, 12, rgb(240, 240, 248));
    draw::fill_rect(fb, x, y, 12, 2, rgb(240, 240, 248));
}

pub fn taskbar_label() -> &'static str {
    match APPS.lock().focus {
        AppId::Console => "Console",
        AppId::Settings => "Settings",
        AppId::Browser => "Browser",
        AppId::Files => "Files",
        AppId::Wifi => "Wi-Fi",
    }
}

pub fn shell_open(name: &str) -> bool {
    let app = match name {
        "console" | "term" => AppId::Console,
        "settings" | "setting" => AppId::Settings,
        "browser" | "firefox" | "web" => AppId::Browser,
        "files" | "file" | "explorer" => AppId::Files,
        "wifi" | "wireless" => AppId::Wifi,
        _ => return false,
    };
    open(app);
    true
}
