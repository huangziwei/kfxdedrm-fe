//! The band above the grid: [`count_label`] left, `out_dir` right.
//!
//! `out_dir` appears nowhere else on the grid screen.

use std::path::Path;

use crate::eink::fb::Framebuffer;
use crate::ui::text::TextRenderer;

/// Gap above the band.
pub const TOP: u32 = 16;
/// Band height, including the bottom rule.
pub const HEIGHT: u32 = 88;
/// Gap below the band.
pub const BOTTOM_GAP: u32 = 16;
/// Headroom `grid::Layout::compute` receives.
pub const MARGIN: u32 = TOP + HEIGHT + BOTTOM_GAP;

const MARGIN_X: u32 = 40;

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
    fb.fill_rect(TOP, 0, xres, HEIGHT, 0xFF);

    let baseline = (TOP + HEIGHT * 55 / 100) as i32;
    renderer.draw(
        fb,
        MARGIN_X as i32,
        baseline,
        &count_label(pending, total),
        false,
    );

    // Right-aligned, clipped from the left: the tail of a path distinguishes
    // one destination from another.
    let dest = format!("→ {}", out_dir.display());
    let w = renderer.measure_width(&dest);
    let x = (xres.saturating_sub(MARGIN_X) as i32 - w as i32).max(MARGIN_X as i32);
    renderer.draw(fb, x, baseline, &dest, false);

    fb.fill_rect(TOP + HEIGHT - 2, MARGIN_X, xres - MARGIN_X * 2, 2, 0x00);
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
