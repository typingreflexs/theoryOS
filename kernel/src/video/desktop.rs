use super::apps;
use super::draw::{self, rgb, text_width};
use super::framebuffer::Framebuffer;
use super::shell;
use crate::sched::timer;

pub const TASKBAR_H: u32 = 40;

const COLOR_TASKBAR: u32 = rgb(28, 28, 36);
const COLOR_TASKBAR_TOP: u32 = rgb(72, 120, 220);
const COLOR_START_BTN: u32 = rgb(48, 48, 60);
const COLOR_TEXT: u32 = rgb(240, 240, 248);
const COLOR_TEXT_DIM: u32 = rgb(160, 160, 176);
const COLOR_GRAD_TOP: u32 = rgb(24, 48, 96);
const COLOR_GRAD_BOT: u32 = rgb(8, 12, 28);

const COLOR_TERM_BG: u32 = rgb(16, 16, 20);
const COLOR_TERM_TITLE: u32 = rgb(36, 36, 48);
const COLOR_TERM_BORDER: u32 = rgb(72, 72, 88);
const COLOR_TERM_TEXT: u32 = rgb(210, 210, 220);
const COLOR_TERM_PROMPT: u32 = rgb(96, 200, 120);

struct ConsoleLayout {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    title_h: u32,
    input_h: u32,
}

pub fn work_height(fb: &Framebuffer) -> u32 {
    fb.height.saturating_sub(TASKBAR_H)
}

pub fn taskbar_y(fb: &Framebuffer) -> u32 {
    work_height(fb)
}

fn console_layout(fb: &Framebuffer) -> ConsoleLayout {
    let margin = 24;
    let wh = work_height(fb);
    let w = fb.width.saturating_sub(margin * 2).max(400).min(720);
    let h = wh.saturating_sub(margin * 2).max(200).min(420);
    let x = fb.width.saturating_sub(w) / 2;
    let y = margin + wh.saturating_sub(h + margin * 2) / 4;
    ConsoleLayout {
        x,
        y,
        w,
        h,
        title_h: 32,
        input_h: 28,
    }
}

pub fn draw_full() {
    let Some(fb) = super::framebuffer() else {
        return;
    };
    draw_background(fb);
    if apps::focus() == apps::AppId::Console {
        draw_console_window(fb);
    }
    apps::draw(fb);
    draw_taskbar(fb);
}

pub fn redraw_all() {
    draw_full();
}

pub fn redraw_console() {
    redraw_all();
}

pub fn update_clock(fb: &Framebuffer) {
    draw_taskbar_clock(fb);
}

fn draw_background(fb: &Framebuffer) {
    draw::fill_gradient(fb, COLOR_GRAD_TOP, COLOR_GRAD_BOT, work_height(fb));
}

fn draw_console_window(fb: &Framebuffer) {
    let win = console_layout(fb);
    draw::fill_rect(fb, win.x, win.y, win.w, win.h, COLOR_TERM_BG);
    draw::fill_rect(fb, win.x, win.y, win.w, win.title_h, COLOR_TERM_TITLE);
    draw::draw_rect_outline(fb, win.x, win.y, win.w, win.h, COLOR_TERM_BORDER);
    draw::draw_text(
        fb,
        win.x + 12,
        win.y + 8,
        "Theory OS Console",
        COLOR_TEXT,
        None,
    );

    let output_y = win.y + win.title_h + 8;
    let output_h = win
        .h
        .saturating_sub(win.title_h + win.input_h + 16);
    draw::fill_rect(
        fb,
        win.x + 8,
        output_y,
        win.w - 16,
        output_h,
        rgb(12, 12, 16),
    );

    let line_h = 18;
    let max_visible = (output_h / line_h).max(1) as usize;
    shell::with_terminal(|term| {
        let start = term.line_count().saturating_sub(max_visible);
        for (vis, idx) in (start..term.line_count()).enumerate() {
            if let Some(line) = term.line(idx) {
                let y = output_y + 4 + vis as u32 * line_h;
                draw::draw_text(fb, win.x + 14, y, line, COLOR_TERM_TEXT, None);
            }
        }

        let input_y = win.y + win.h - win.input_h - 6;
        draw::fill_rect(
            fb,
            win.x + 8,
            input_y,
            win.w - 16,
            win.input_h,
            rgb(8, 8, 12),
        );
        draw::draw_text(fb, win.x + 14, input_y + 6, "$ ", COLOR_TERM_PROMPT, None);
        let prompt_w = text_width("$ ");
        draw::draw_text(
            fb,
            win.x + 14 + prompt_w,
            input_y + 6,
            term.input_text(),
            COLOR_TERM_TEXT,
            None,
        );
    });
}

fn draw_taskbar(fb: &Framebuffer) {
    let ty = taskbar_y(fb);
    draw::fill_rect(fb, 0, ty, fb.width, 1, COLOR_TASKBAR_TOP);
    draw::fill_rect(fb, 0, ty + 1, fb.width, TASKBAR_H - 1, COLOR_TASKBAR);

    let btn_w = 120;
    let btn_h = TASKBAR_H - 12;
    let btn_y = ty + 6;
    draw::fill_rect(fb, 8, btn_y, btn_w, btn_h, COLOR_START_BTN);
    draw::draw_rect_outline(fb, 8, btn_y, btn_w, btn_h, rgb(80, 80, 96));
    draw::draw_text(fb, 20, btn_y + 8, "Start", COLOR_TEXT, None);

    let label = apps::taskbar_label();
    draw::draw_text(fb, 140, btn_y + 8, label, COLOR_TEXT_DIM, None);
    draw_taskbar_clock(fb);
}

fn draw_taskbar_clock(fb: &Framebuffer) {
    let ty = taskbar_y(fb);
    let mut buf = [0u8; 9];
    let text = format_clock(&mut buf);
    let tw = text_width(text);
    let x = fb.width.saturating_sub(tw + 24);
    let bg_y = ty + 6;
    draw::fill_rect(fb, x.saturating_sub(8), bg_y, tw + 16, 26, rgb(40, 40, 52));
    draw::draw_text(fb, x, bg_y + 5, text, COLOR_TEXT, None);
}

fn format_clock(buf: &mut [u8; 9]) -> &str {
    let ns = timer::monotonic_ns();
    let total_sec = ns / 1_000_000_000;
    let h = (total_sec / 3600) % 24;
    let m = (total_sec / 60) % 60;
    let s = total_sec % 60;
    buf[0] = b'0' + (h / 10) as u8;
    buf[1] = b'0' + (h % 10) as u8;
    buf[2] = b':';
    buf[3] = b'0' + (m / 10) as u8;
    buf[4] = b'0' + (m % 10) as u8;
    buf[5] = b':';
    buf[6] = b'0' + (s / 10) as u8;
    buf[7] = b'0' + (s % 10) as u8;
    buf[8] = 0;
    unsafe { core::str::from_utf8_unchecked(&buf[..8]) }
}
