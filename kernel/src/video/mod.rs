//! Linear framebuffer graphics — desktop UI and user-space mapping.

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

pub fn update_clock() {
    if let Some(fb) = FRAMEBUFFER.get() {
        desktop::update_clock(fb);
    }
}

pub fn proc_fb_line() -> alloc::string::String {
    user_map::proc_fb_line()
}
