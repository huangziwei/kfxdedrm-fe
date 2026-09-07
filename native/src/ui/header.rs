//! The band above the grid: [`count_label`] left, `Config::out_dir` right.
//!
//! [`draw`] is the only place the grid screen names `Config::out_dir`.

use std::path::Path;

use crate::eink::fb::Framebuffer;
use crate::ui::scale::Scale;
use crate::ui::text::TextRenderer;

/// Gap above the band.
const TOP: u32 = 16;
/// Band height, including the bottom rule.
const HEIGHT: u32 = 88;
/// Gap below the band.
const BOTTOM_GAP: u32 = 16;

const MARGIN_X: u32 = 40;
/// The bottom rule's thickness.
const RULE_H: u32 = 2;

/// Headroom `grid::Layout::compute` receives, on a panel `fb_xres` wide.
pub fn margin(fb_xres: u32) -> u32 {
    let s = Scale::of_width(fb_xres);
    s.px(TOP) + s.px(HEIGHT) + s.px(BOTTOM_GAP)
}

/// `pending` and `total` as a phrase. `pending` is what `app::decrypt_all`
/// acts on.
fn count_label(pending: usize, total: usize) -> String {
    match (pending, total) {
        (0, 0) => "No books found".to_string(),
        // Reachable with `Config::show_done` on.
        (0, _) => "All decrypted".to_string(),
        (1, _) => "1 book to decrypt".to_string(),
        (n, _) => format!("{n} books to decrypt"),
    }
}

/// [`count_label`] left, `out_dir` right, a rule under both.
pub fn draw(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    pending: usize,
    total: usize,
    out_dir: &Path,
) {
    let xres = fb.var.xres;
    let s = Scale::of_width(xres);
    let (top, height, margin_x) = (s.px(TOP), s.px(HEIGHT), s.px(MARGIN_X));
    fb.fill_rect(top, 0, xres, height, 0xFF);

    let baseline = (top + height * 55 / 100) as i32;
    renderer.draw(
        fb,
        margin_x as i32,
        baseline,
        &count_label(pending, total),
        false,
    );

    // Right-aligned, and the only place the destination is named.
    let dest = format!("→ {}", out_dir.display());
    let w = renderer.measure_width(&dest);
    let x = (xres.saturating_sub(margin_x) as i32 - w as i32).max(margin_x as i32);
    renderer.draw(fb, x, baseline, &dest, false);

    let rule_h = s.px(RULE_H);
    fb.fill_rect(
        top + height - rule_h,
        margin_x,
        xres.saturating_sub(margin_x * 2),
        rule_h,
        0x00,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_reads_as_work_remaining() {
        assert_eq!(count_label(0, 0), "No books found");
        // `Config::show_done` on, everything done.
        assert_eq!(count_label(0, 4), "All decrypted");
        assert_eq!(count_label(1, 1), "1 book to decrypt");
        assert_eq!(count_label(7, 9), "7 books to decrypt");
    }
}
