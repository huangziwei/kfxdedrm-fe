//! The grid's control bar, a [`crate::ui::strip`] along the bottom of the
//! panel. Slots left to right:
//!
//! - `[ Exit ]`.
//! - `[ Settings ]`.
//! - `[ Decrypt All (N) ]`, live while `pending` is non-zero.
//! - `← Prev / N / Next →` across the remaining width, live while
//!   `total_pages` exceeds 1. The only paging on a device with no bezel keys.
//!   Paging is not an action on the page, so it takes no slot of its own and
//!   no brackets.
//!
//! [`Zones`] places the four against the panel width and [`hit`] answers from
//! the same placement [`draw`] draws at, which is what keeps a label and the
//! zone under it in step. Both are integer geometry; only [`draw`] measures
//! text, to pick the one [`LABEL_PX`] every label fits its zone at.

use crate::eink::fb::Framebuffer;
use crate::ui::scale::Scale;
use crate::ui::strip;
use crate::ui::text::TextRenderer;

/// Bar height on a panel `fb_xres` wide, from [`strip::H`].
pub fn strip_h(fb_xres: u32) -> u32 {
    strip::h(fb_xres)
}

const EXIT_ZONE_W: u32 = 180;
/// Right of [`EXIT_ZONE_W`].
const SETTINGS_ZONE_W: u32 = 240;
/// Right of [`SETTINGS_ZONE_W`]. Widest: its label carries a count.
const ALL_ZONE_W: u32 = 300;
/// The three named zones at the widths above.
const FIXED_W: u32 = EXIT_ZONE_W + SETTINGS_ZONE_W + ALL_ZONE_W;

/// What paging keeps at the right of the strip whatever the named zones ask
/// for: two halves a thumb can tell apart.
const NAV_MIN_W: u32 = 240;
/// The least the named zones shrink to. Below this `Exit` stops being
/// tappable, and a user who cannot leave is worse off than one who cannot page.
const FIXED_MIN_W: u32 = 300;

/// Text inset from a zone's left edge, on a zone wide enough for it.
const LABEL_INSET: u32 = 32;

/// The slot labels. An action is bracketed; paging only moves through the
/// pages it reports and is not.
const EXIT: &str = "[ Exit ]";
const SETTINGS: &str = "[ Settings ]";
const PREV: &str = "← Prev";
const NEXT: &str = "Next →";

/// Label sizes the bar drops through, largest first, until every label fits
/// the zone it is drawn in.
const LABEL_PX: &[f32] = &[32.0, 28.0, 24.0, 20.0, 17.0];

/// Reference panel width for the layouts checked against one.
pub const NARROWEST_PANEL_W: u32 = 1072;

/// Where the toolbar's zones start, for a panel `fb_xres` wide.
///
/// The named zones hold [`EXIT_ZONE_W`] / [`SETTINGS_ZONE_W`] / [`ALL_ZONE_W`]
/// while that leaves [`NAV_MIN_W`] for paging, and shrink together, in
/// proportion, where it does not. Widths that do not scale put `nav` past the
/// right edge of a 758-wide panel, leaving no room to page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zones {
    /// Left edge of `Settings`. `Exit` runs from 0 to here.
    pub settings: u32,
    /// Left edge of `Decrypt All`.
    pub all: u32,
    /// Left edge of the page-nav region.
    pub nav: u32,
    /// Where `Prev` gives way to `Next`.
    pub nav_mid: u32,
}

impl Zones {
    pub fn compute(fb_xres: u32) -> Self {
        let s = Scale::of_width(fb_xres);
        let (exit_w, settings_w, all_w) =
            (s.px(EXIT_ZONE_W), s.px(SETTINGS_ZONE_W), s.px(ALL_ZONE_W));
        let fixed_w = exit_w + settings_w + all_w;
        let room = fb_xres
            .saturating_sub(s.px(NAV_MIN_W))
            .clamp(s.px(FIXED_MIN_W).min(fixed_w), fixed_w);
        let settings = exit_w * room / fixed_w;
        let all = settings + settings_w * room / fixed_w;
        // `min`: a panel narrower than [`FIXED_MIN_W`] plus [`NAV_MIN_W`] has
        // no nav region, and `hit` answers `None` past the named zones.
        let nav = (all + all_w * room / fixed_w).min(fb_xres);
        Self {
            settings,
            all,
            nav,
            nav_mid: (nav + fb_xres) / 2,
        }
    }

    /// Width of the zone starting at `left` and ending at `right`.
    fn width(left: u32, right: u32) -> u32 {
        right.saturating_sub(left)
    }
}

/// The inset a `zone_w`-wide zone can afford, so a narrow zone still shows its
/// label rather than pushing it out.
fn inset_for(zone_w: u32, s: Scale) -> u32 {
    (zone_w / 6).min(s.px(LABEL_INSET))
}

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

pub fn strip_top(fb_xres: u32, fb_yres: u32) -> u32 {
    strip::top(fb_xres, fb_yres)
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
    if ty < strip_top(fb_xres, fb_yres) {
        return None;
    }
    let z = Zones::compute(fb_xres);
    if tx < z.settings {
        return Some(PagerHit::Exit);
    }
    if tx < z.all {
        return Some(PagerHit::Settings);
    }
    if tx < z.nav {
        // `draw` leaves this zone empty at `pending == 0`.
        return (pending > 0).then_some(PagerHit::DecryptAll);
    }
    if total_pages <= 1 {
        return None;
    }
    // Splits `nav`..`fb_xres`, which `Zones` keeps at [`NAV_MIN_W`] wherever
    // the panel affords it.
    if tx < z.nav_mid {
        Some(PagerHit::Prev)
    } else {
        Some(PagerHit::Next)
    }
}

/// The largest [`LABEL_PX`] at which every label fits the zone it is drawn in,
/// or the smallest of them. `draw` is the only caller; the labels are passed as
/// `(text, zone width)`.
fn label_px(renderer: &mut TextRenderer, labels: &[(&str, u32)], s: Scale) -> f32 {
    for px in LABEL_PX {
        let scaled = s.font(*px);
        let fits = renderer.at_px(scaled, |r| {
            labels
                .iter()
                .all(|(text, zone_w)| r.measure_width(text) + inset_for(*zone_w, s) * 2 <= *zone_w)
        });
        if fits {
            return scaled;
        }
    }
    s.font(LABEL_PX[LABEL_PX.len() - 1])
}

pub fn draw(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    page: usize,
    total_pages: usize,
    pending: usize,
) {
    let xres = fb.var.xres;
    let s = Scale::of_width(xres);
    let strip_y = strip::base(fb);
    let baseline = strip::baseline(xres, strip_y);
    let z = Zones::compute(xres);
    let all_label = format!("[ Decrypt All ({pending}) ]");
    let counter = format!("{} / {}", page + 1, total_pages);

    // One size for the bar, so every label sits on the same baseline at the
    // same weight. The nav labels share their region, hence the halves.
    let nav_half = Zones::width(z.nav, xres) / 2;
    let px = label_px(
        renderer,
        &[
            (EXIT, Zones::width(0, z.settings)),
            (SETTINGS, Zones::width(z.settings, z.all)),
            (&all_label, Zones::width(z.all, z.nav)),
            (PREV, nav_half),
            (NEXT, nav_half),
        ],
        s,
    );

    renderer.at_px(px, |r| {
        strip::label(fb, r, 0, z.settings, baseline, EXIT, false);
        strip::label(
            fb,
            r,
            z.settings,
            Zones::width(z.settings, z.all),
            baseline,
            SETTINGS,
            false,
        );
        // `TextRenderer::draw` has black and white and no grey to dim a label
        // with, and [`hit`] returns `None` at `pending == 0`.
        if pending > 0 {
            strip::label(
                fb,
                r,
                z.all,
                Zones::width(z.all, z.nav),
                baseline,
                &all_label,
                false,
            );
        }

        if total_pages <= 1 {
            return;
        }
        // Paging keeps the open region the slots do not divide: each label sits
        // inside the half `hit` answers for it, a `Next` drawn left of
        // `nav_mid` reading as `Prev` under the finger.
        let inset = inset_for(nav_half, s);
        if page > 0 {
            r.draw(fb, (z.nav + inset) as i32, baseline, PREV, false);
        }
        let next_w = r.measure_width(NEXT);
        let next_x = xres
            .saturating_sub(inset)
            .saturating_sub(next_w)
            .max(z.nav_mid + inset);
        if page + 1 < total_pages {
            r.draw(fb, next_x as i32, baseline, NEXT, false);
        }
        // Centred in the nav region, and dropped where either label reaches it.
        let counter_w = r.measure_width(&counter);
        let counter_x = z.nav + Zones::width(z.nav, xres) / 2 - counter_w / 2;
        let clear_left = page == 0 || counter_x > z.nav + inset + r.measure_width(PREV);
        let clear_right = page + 1 >= total_pages || counter_x + counter_w < next_x;
        if clear_left && clear_right {
            r.draw(fb, counter_x as i32, baseline, &counter, false);
        }
    });

    // Separators last: a label that ran long is cut by its own zone edge
    // rather than reading as part of the next one.
    //
    // One to the left of each *live* zone, so a dead zone leaves no empty
    // compartment behind: with nothing to decrypt there is no `Decrypt All`
    // label, and a bar on either side of the gap would read as a button that
    // lost its text.
    for (edge, live) in [
        (z.settings, true),
        (z.all, pending > 0),
        (z.nav, total_pages > 1),
    ] {
        if live {
            strip::separator(fb, edge);
        }
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
        hit(x, YRES - strip_h(XRES) / 2, XRES, YRES, pages, pending)
    }

    /// Every panel width a shipped device's X server reports.
    const PANELS: &[u32] = &[600, 758, 1072, 1236, 1264, 1860];

    #[test]
    fn every_fixed_zone_is_reachable_on_the_narrowest_panel() {
        let z = Zones::compute(XRES);
        assert_eq!(tap(20, 1, 3), Some(PagerHit::Exit));
        assert_eq!(tap(z.settings + 20, 1, 3), Some(PagerHit::Settings));
        assert_eq!(tap(z.all + 20, 1, 3), Some(PagerHit::DecryptAll));
    }

    #[test]
    fn nav_splits_its_own_region_rather_than_the_screen() {
        let z = Zones::compute(XRES);
        // The screen midpoint sits inside the fixed zones at this width.
        assert_eq!(tap(z.nav + 10, 3, 1), Some(PagerHit::Prev));
        assert_eq!(tap(XRES - 10, 3, 1), Some(PagerHit::Next));
    }

    #[test]
    fn a_single_page_has_no_nav_zone() {
        let z = Zones::compute(XRES);
        assert_eq!(tap(z.nav + 10, 1, 1), None);
        assert_eq!(tap(XRES - 10, 1, 1), None);
    }

    #[test]
    fn decrypt_all_is_dead_when_there_is_nothing_left_to_decrypt() {
        let z = Zones::compute(XRES);
        assert_eq!(tap(z.all + 20, 1, 0), None);
        // The zones around it still work — only that one goes inert.
        assert_eq!(tap(z.settings + 20, 1, 0), Some(PagerHit::Settings));
    }

    /// Every panel keeps five reachable zones, and paging keeps at least
    /// [`NAV_MIN_W`] of the strip.
    #[test]
    fn every_panel_keeps_a_tappable_zone_for_each_control() {
        for xres in PANELS {
            let z = Zones::compute(*xres);
            let s = Scale::of_width(*xres);
            let at = |x: u32| hit(x, YRES - strip_h(*xres) / 2, *xres, YRES, 3, 3);
            assert_eq!(at(0), Some(PagerHit::Exit), "exit at {xres}");
            assert_eq!(
                at(z.settings),
                Some(PagerHit::Settings),
                "settings at {xres}"
            );
            assert_eq!(at(z.all), Some(PagerHit::DecryptAll), "all at {xres}");
            assert_eq!(at(z.nav), Some(PagerHit::Prev), "prev at {xres}");
            assert_eq!(at(*xres - 1), Some(PagerHit::Next), "next at {xres}");
            // The floor is a design pixel, so the comparison is too.
            let kept = xres - z.nav;
            assert!(
                kept >= s.px(NAV_MIN_W),
                "{xres}: paging kept {kept} < {}",
                s.px(NAV_MIN_W)
            );
        }
    }

    /// The zones rise with the panel and never cross each other.
    #[test]
    fn the_zones_stay_in_order_and_inside_the_panel() {
        for xres in PANELS {
            let z = Zones::compute(*xres);
            assert!(0 < z.settings, "{xres}");
            assert!(z.settings < z.all, "{xres}");
            assert!(z.all < z.nav, "{xres}");
            assert!(z.nav < z.nav_mid, "{xres}");
            assert!(z.nav_mid < *xres, "{xres}");
        }
    }

    /// A panel wide enough for all three keeps the widths the layout was drawn
    /// at; only a narrow one scales them down.
    #[test]
    fn a_wide_panel_keeps_the_full_zone_widths() {
        for xres in [1072u32, 1236, 1264, 1860] {
            let z = Zones::compute(xres);
            assert_eq!(z.settings, EXIT_ZONE_W, "{xres}");
            assert_eq!(z.all, EXIT_ZONE_W + SETTINGS_ZONE_W, "{xres}");
            assert_eq!(z.nav, FIXED_W, "{xres}");
        }
        // 758 cannot afford them and shrinks all three together.
        let narrow = Zones::compute(758);
        assert!(narrow.nav < FIXED_W);
        assert!(narrow.settings < EXIT_ZONE_W);
    }

    /// A panel too narrow for the named zones plus [`NAV_MIN_W`] keeps `Exit`
    /// rather than dividing what is left evenly.
    #[test]
    fn a_panel_with_no_room_still_opens_the_way_out() {
        let z = Zones::compute(320);
        assert_eq!(z.settings, EXIT_ZONE_W * FIXED_MIN_W / FIXED_W);
        assert!(z.nav <= 320);
        assert_eq!(hit(0, YRES - 1, 320, YRES, 3, 3), Some(PagerHit::Exit));
    }

    #[test]
    fn taps_above_the_strip_belong_to_the_grid() {
        assert_eq!(hit(20, strip_top(XRES, YRES) - 1, XRES, YRES, 3, 3), None);
    }

    /// A zone narrower than six insets takes a smaller one rather than pushing
    /// its label out of the zone.
    #[test]
    fn a_narrow_zone_gives_up_its_inset() {
        let s = Scale::none();
        assert_eq!(inset_for(600, s), LABEL_INSET);
        assert_eq!(inset_for(192, s), LABEL_INSET);
        assert_eq!(inset_for(90, s), 15);
        assert_eq!(inset_for(0, s), 0);
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
