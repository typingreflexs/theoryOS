//! Linear framebuffer graphics — desktop UI, apps, and user-space fb mapping.
//!
//! # Submodules
//! - `framebuffer` — Limine linear FB, pixel format, blit helpers
//! - `desktop`     — Wallpaper, taskbar, clock
//! - `apps`        — Window manager, launcher, click handling
//! - `browser`     — Async HTTP page loader (text-only)
//! - `shell`       — In-kernel terminal emulator
//! - `draw` / `font` — Primitives and 8×16 bitmap font
//! - `user_map`    — `/proc/fb` metadata for userspace mmap
//!
//! The idle thread calls `poll_input()` every iteration to handle keyboard/mouse.

pub mod apps;
pub mod browser;
pub mod desktop;
mod draw;
mod font;
mod framebuffer;
mod shell;
mod user_map;

use spin::Once;

use crate::console::Console;

pub use framebuffer::{Framebuffer, UserFbMeta, FB_MAGIC};
pub use user_map::map_into_user;

static FRAMEBUFFER: Once<Framebuffer> = Once::new();

/// Initialize PS/2 input, framebuffer, apps, and draw the first frame.
pub fn init() {
    crate::input::init();
    let Some(fb) = Framebuffer::from_boot() else {
        Console::println("[video] no framebuffer from Limine");
        return;
    };
    let _ = FRAMEBUFFER.call_once(|| fb);
    crate::input::mouse::set_bounds(fb.width, fb.height);
    apps::init();
    browser::init();
    shell::init();
    Console::println("[video] framebuffer online");
    desktop::draw_full();
}

pub fn framebuffer() -> Option<&'static Framebuffer> {
    FRAMEBUFFER.get()
}

pub fn is_available() -> bool {
    FRAMEBUFFER.get().is_some()
}

/// Poll keyboard/mouse and redraw if any app or the cursor changed.
pub fn poll_input() {
    crate::input::poll_devices(|key| {
        match apps::focus() {
            apps::AppId::Console => shell::handle_key(key),
            apps::AppId::Browser => browser::handle_key(key),
            _ => {}
        }
    });

    let mouse_redraw = crate::input::mouse::take_moved();
    let mouse_click = crate::input::mouse::take_clicked();

    if mouse_click {
        if let Some(fb) = FRAMEBUFFER.get() {
            let m = crate::input::mouse::state();
            apps::handle_click(m.x, m.y, fb);
        }
    }

    if shell::take_dirty()
        || apps::take_dirty()
        || browser::take_dirty()
        || mouse_redraw
        || mouse_click
    {
        desktop::redraw_all();
    } else if browser::is_loading() {
        desktop::redraw_all();
    }
}

/// Update taskbar clock (called ~every 500 ms from idle loop).
pub fn update_clock() {
    if let Some(fb) = FRAMEBUFFER.get() {
        desktop::update_clock(fb);
    }
}

/// One-line summary for `/proc/fb` (width, height, bpp, address).
pub fn proc_fb_line() -> alloc::string::String {
    user_map::proc_fb_line()
}
