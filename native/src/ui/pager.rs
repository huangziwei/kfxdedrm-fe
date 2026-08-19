//! [`STRIP_H`] tall, along the bottom of the panel. Zones left to right:
//!
//! - `Exit`, [`EXIT_ZONE_W`] wide.
//! - `Settings`, [`SETTINGS_ZONE_W`] wide.
//! - `Decrypt All (N)`, [`ALL_ZONE_W`] wide, live while `pending` is non-zero.
//! - `← Prev / N / Next →` across the remaining width, live while
//!   `total_pages` exceeds 1. The only paging on a device with no bezel keys.

use crate::eink::fb::Framebuffer;
use crate::ui::text::TextRenderer;

pub const STRIP_H: u32 = 80;

const EXIT_ZONE_W: u32 = 180;
/// Right of [`EXIT_ZONE_W`].
const SETTINGS_ZONE_W: u32 = 240;
/// Right of [`SETTINGS_ZONE_W`]. Widest: its label carries a count.
const ALL_ZONE_W: u32 = 300;
/// Left edge of the Decrypt-All zone.
const ALL_LEFT: u32 = EXIT_ZONE_W + SETTINGS_ZONE_W;
/// Left edge of the page-nav region.
const NAV_LEFT: u32 = EXIT_ZONE_W + SETTINGS_ZONE_W + ALL_ZONE_W;

/// Text inset from a zone's left edge.
const LABEL_INSET: i32 = 32;

/// Paperwhite-class panel width, the narrowest the layout runs on.
pub const NARROWEST_PANEL_W: u32 = 1072;

/// The fixed zones leave [`hit`] two tappable halves at
/// [`NARROWEST_PANEL_W`]. A wider zone breaks paging there alone.
const _: () = assert!(
    NAV_LEFT + 200 < NARROWEST_PANEL_W,
    "toolbar zones leave too little room for page navigation"
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagerHit {
    Exit,
    Settings,
    /// Returned while `pending` is non-zero.
    DecryptAll,
    /// The nav region's left and right half.
    Prev,
    Next,
}

pub fn n_pages(books: usize, page_size: usize) -> usize {
    // Outer `.max(1)`: an empty grid holds one page. Inner: no divide by zero.
    books.div_ceil(page_size.max(1)).max(1)
}

pub fn strip_top(fb_yres: u32) -> u32 {
    fb_yres.saturating_sub(STRIP_H)
}

/// The zone at `(tx, ty)`. Integer geometry, no framebuffer.
pub fn hit(
    tx: u32,
    ty: u32,
    fb_xres: u32,
    fb_yres: u32,
    total_pages: usize,
    pending: usize,
) -> Option<PagerHit> {
    if ty < strip_top(fb_yres) {
        return None;
    }
    if tx < EXIT_ZONE_W {
        return Some(PagerHit::Exit);
    }
    if tx < ALL_LEFT {
        return Some(PagerHit::Settings);
    }
    if tx < NAV_LEFT {
        // `draw` leaves this zone empty at `pending == 0`.
        return (pending > 0).then_some(PagerHit::DecryptAll);
    }
    if total_pages <= 1 {
        return None;
    }
    // Splits NAV_LEFT..fb_xres. The screen midpoint can land left of
    // NAV_LEFT, leaving Prev a sliver.
    let nav_mid = (NAV_LEFT + fb_xres) / 2;
    if tx < nav_mid {
        Some(PagerHit::Prev)
    } else {
        Some(PagerHit::Next)
    }
}

pub fn draw(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    page: usize,
    total_pages: usize,
    pending: usize,
) {
    let strip_y = strip_top(fb.var.yres);
    // 2px black divider, white strip body below.
    fb.fill_rect(strip_y, 0, fb.var.xres, 2, 0x00);
    fb.fill_rect(strip_y + 2, 0, fb.var.xres, STRIP_H - 2, 0xFF);

    let baseline = (strip_y + STRIP_H * 70 / 100) as i32;

    renderer.draw(fb, LABEL_INSET, baseline, "Exit", false);
    fb.fill_rect(strip_y + 12, EXIT_ZONE_W - 2, 2, STRIP_H - 24, 0x00);

    renderer.draw(
        fb,
        EXIT_ZONE_W as i32 + LABEL_INSET,
        baseline,
        "Settings",
        false,
    );
    fb.fill_rect(strip_y + 12, ALL_LEFT - 2, 2, STRIP_H - 24, 0x00);

    // `TextRenderer::draw` has black and white and no grey to dim a label
    // with, and [`hit`] returns `None` at `pending == 0`.
    if pending > 0 {
        renderer.draw(
            fb,
            ALL_LEFT as i32 + LABEL_INSET,
            baseline,
            &format!("Decrypt All ({pending})"),
            false,
        );
    }
    fb.fill_rect(strip_y + 12, NAV_LEFT - 2, 2, STRIP_H - 24, 0x00);

    if total_pages <= 1 {
        return;
    }

    let label_mid = format!("{} / {}", page + 1, total_pages);

    // Each label appears while that direction exists.
    if page > 0 {
        renderer.draw(fb, NAV_LEFT as i32 + LABEL_INSET, baseline, "← Prev", false);
    }
    // Centered in NAV_LEFT..xres. Screen-centering lands it on the last
    // separator at these zone widths.
    let mid_w = renderer.measure_width(&label_mid);
    let mid_x = (NAV_LEFT as i32 + fb.var.xres as i32) / 2 - mid_w as i32 / 2;
    renderer.draw(fb, mid_x, baseline, &label_mid, false);
    if page + 1 < total_pages {
        let next_w = renderer.measure_width("Next →");
        let next_x = fb.var.xres as i32 - LABEL_INSET * 2 - next_w as i32;
        renderer.draw(fb, next_x, baseline, "Next →", false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`NARROWEST_PANEL_W`].
    const XRES: u32 = NARROWEST_PANEL_W;
    const YRES: u32 = 1448;

    /// A tap at `x`, vertically centered in the strip.
    fn tap(x: u32, pages: usize, pending: usize) -> Option<PagerHit> {
        hit(x, YRES - STRIP_H / 2, XRES, YRES, pages, pending)
    }

    #[test]
    fn every_fixed_zone_is_reachable_on_the_narrowest_panel() {
        assert_eq!(tap(20, 1, 3), Some(PagerHit::Exit));
        assert_eq!(tap(EXIT_ZONE_W + 20, 1, 3), Some(PagerHit::Settings));
        assert_eq!(tap(ALL_LEFT + 20, 1, 3), Some(PagerHit::DecryptAll));
    }

    #[test]
    fn nav_splits_its_own_region_rather_than_the_screen() {
        // The screen midpoint sits inside the fixed zones at this width.
        assert_eq!(tap(NAV_LEFT + 10, 3, 1), Some(PagerHit::Prev));
        assert_eq!(tap(XRES - 10, 3, 1), Some(PagerHit::Next));
    }

    #[test]
    fn a_single_page_has_no_nav_zone() {
        assert_eq!(tap(NAV_LEFT + 10, 1, 1), None);
        assert_eq!(tap(XRES - 10, 1, 1), None);
    }

    #[test]
    fn decrypt_all_is_dead_when_there_is_nothing_left_to_decrypt() {
        assert_eq!(tap(ALL_LEFT + 20, 1, 0), None);
        // The zones around it still work — only that one goes inert.
        assert_eq!(tap(EXIT_ZONE_W + 20, 1, 0), Some(PagerHit::Settings));
    }

    #[test]
    fn taps_above_the_strip_belong_to_the_grid() {
        assert_eq!(hit(20, strip_top(YRES) - 1, XRES, YRES, 3, 3), None);
    }

    #[test]
    fn an_empty_grid_still_has_one_page() {
        assert_eq!(n_pages(0, 8), 1);
        assert_eq!(n_pages(8, 8), 1);
        assert_eq!(n_pages(9, 8), 2);
        // A degenerate layout must not divide by zero.
        assert_eq!(n_pages(5, 0), 5);
    }
}
