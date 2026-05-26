use super::font;
use super::framebuffer::Framebuffer;

/// 32-bit BGRx pixel constant (matches [`Framebuffer::pack_color`] for BGR).
pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    u32::from_le_bytes([b, g, r, 0xFF])
}

fn map_point(fb: &Framebuffer, x: u32, y: u32) -> (u32, u32) {
    let px = if fb.flip_x {
        fb.width.saturating_sub(1).saturating_sub(x)
    } else {
        x
    };
    let py = if fb.flip_y {
        fb.height.saturating_sub(1).saturating_sub(y)
    } else {
        y
    };
    (px, py)
}

fn normalize_color(fb: &Framebuffer, color: u32) -> u32 {
    match fb.format {
        super::framebuffer::PixelFormat::Bgr => color,
        super::framebuffer::PixelFormat::Rgb => {
            let c = color.to_le_bytes();
            u32::from_le_bytes([c[2], c[1], c[0], c[3]])
        }
    }
}

pub fn put_pixel(fb: &Framebuffer, x: u32, y: u32, color: u32) {
    if x >= fb.width || y >= fb.height {
        return;
    }
    let (px, py) = map_point(fb, x, y);
    let stride = fb.pitch / 4;
    // SAFETY: mapped framebuffer, bounds checked above (including mapped coords).
    unsafe {
        fb.pixels()
            .add(py as usize * stride as usize + px as usize)
            .write(normalize_color(fb, color));
    }
}

pub fn fill_rect(fb: &Framebuffer, x: u32, y: u32, w: u32, h: u32, color: u32) {
    if w == 0 || h == 0 {
        return;
    }
    let w = w.min(fb.width.saturating_sub(x));
    let h = h.min(fb.height.saturating_sub(y));
    for row in 0..h {
        for col in 0..w {
            put_pixel(fb, x + col, y + row, color);
        }
    }
}

pub fn fill_gradient(fb: &Framebuffer, top: u32, bottom: u32, max_height: u32) {
    let h = max_height.min(fb.height);
    if h == 0 {
        return;
    }
    for y in 0..h {
        let t = y as u64 * 255 / h as u64;
        let color = lerp_color(top, bottom, t as u8);
        for x in 0..fb.width {
            put_pixel(fb, x, y, color);
        }
    }
}

fn lerp_color(a: u32, b: u32, t: u8) -> u32 {
    let ta = a.to_le_bytes();
    let tb = b.to_le_bytes();
    let out = [
        lerp_u8(ta[0], tb[0], t),
        lerp_u8(ta[1], tb[1], t),
        lerp_u8(ta[2], tb[2], t),
        0xFF,
    ];
    u32::from_le_bytes(out)
}

fn lerp_u8(a: u8, b: u8, t: u8) -> u8 {
    ((a as u16 * (255 - t as u16) + b as u16 * t as u16) / 255) as u8
}

pub fn draw_rect_outline(fb: &Framebuffer, x: u32, y: u32, w: u32, h: u32, color: u32) {
    if w < 2 || h < 2 {
        return;
    }
    fill_rect(fb, x, y, w, 1, color);
    fill_rect(fb, x, y + h - 1, w, 1, color);
    fill_rect(fb, x, y, 1, h, color);
    fill_rect(fb, x + w - 1, y, 1, h, color);
}

pub fn draw_text(fb: &Framebuffer, x: u32, y: u32, text: &str, fg: u32, bg: Option<u32>) {
    let gw = font::glyph_width();
    let mut cx = x;
    for ch in text.bytes() {
        draw_glyph(fb, cx, y, ch, fg, bg);
        cx = cx.saturating_add(gw);
        if cx >= fb.width {
            break;
        }
    }
}

pub fn draw_glyph(fb: &Framebuffer, x: u32, y: u32, ch: u8, fg: u32, bg: Option<u32>) {
    let rows = font::glyph(ch);
    let gw = font::glyph_width();
    let gh = font::glyph_height();
    for (row, bits) in rows.iter().enumerate().take(gh as usize) {
        let py = y + row as u32;
        if py >= fb.height {
            break;
        }
        for col in 0..gw {
            let px = x + col;
            if px >= fb.width {
                break;
            }
            // Bochs/Limine linear FB: bit 0 is the leftmost pixel (not VGA bit 7).
            let on = (bits >> col) & 1 != 0;
            let color = if on { fg } else { bg.unwrap_or(0) };
            if on || bg.is_some() {
                put_pixel(fb, px, py, color);
            }
        }
    }
}

pub fn text_width(text: &str) -> u32 {
    text.len() as u32 * font::glyph_width()
}
