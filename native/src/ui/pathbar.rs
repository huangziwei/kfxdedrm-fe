//! The path field at the top of the `keyboard` overlay: a pill holding
//! `Config::out_dir` as it is typed, with a `✕` zone clearing it.

use crate::eink::fb::Framebuffer;
use crate::ui::grid;
use crate::ui::text::TextRenderer;

/// Geometry, shared by [`clear_hit`] and [`draw`].
pub const TOP: u32 = 16;
pub const HEIGHT: u32 = 88;
pub const MARGIN_X: u32 = 40;
/// Right-hand zone clearing the path, live under `path_active`.
pub const CLEAR_W: u32 = 150;

/// Pill width: the full span between the side margins.
pub fn field_w(xres: u32) -> u32 {
    xres.saturating_sub(MARGIN_X * 2)
}

/// Where the path text starts, clear of the pill's left rounded end.
fn text_x(x: u32) -> i32 {
    (x + HEIGHT / 2 + 6) as i32
}

/// Whether `(tx, ty)` lands on the `✕` zone. `path_active` is what enables it.
pub fn clear_hit(tx: u32, ty: u32, xres: u32, path_active: bool) -> bool {
    if !path_active || !(TOP..TOP + HEIGHT).contains(&ty) {
        return false;
    }
    let (x, w) = (MARGIN_X, field_w(xres));
    (x..x + w).contains(&tx) && tx >= x + w - CLEAR_W
}

/// A rounded pill, the path or the placeholder, and an `✕` under a set path.
pub fn draw(fb: &mut Framebuffer, renderer: &mut TextRenderer, path: &str) {
    let xres = fb.var.xres;
    let x = MARGIN_X;
    let w = field_w(xres);
    let cy = (TOP + HEIGHT / 2) as i32;
    let baseline = (TOP + HEIGHT * 62 / 100) as i32;
    let text_x = text_x(x);

    grid::stroke_round_rect(fb, x as i32, TOP as i32, w, HEIGHT, HEIGHT / 2, 3, 0x00);

    if path.is_empty() {
        renderer.draw(fb, text_x, baseline, "/mnt/us/…", false);
        return;
    }
    // Tail-first: the end of a path is what differs between two of them.
    let right_limit = (x + w).saturating_sub(CLEAR_W) as i32;
    let avail = (right_limit - text_x).max(0) as u32;
    let shown = clamp_tail(renderer, path, avail);
    renderer.draw(fb, text_x, baseline, &shown, false);
    let clear_cx = (x + w).saturating_sub(CLEAR_W / 2) as i32;
    grid::draw_x(fb, clear_cx, cy, 15, 0x00);
}

/// The trailing substring of `s` fitting `max_width`.
fn clamp_tail(renderer: &mut TextRenderer, s: &str, max_width: u32) -> String {
    if renderer.measure_width(s) <= max_width {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut start = 0;
    while start < chars.len() {
        let tail: String = chars[start..].iter().collect();
        if renderer.measure_width(&tail) <= max_width {
            return tail;
        }
        start += 1;
    }
    String::new()
}
