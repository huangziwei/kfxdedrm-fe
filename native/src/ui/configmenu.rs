//! One [`Row`] per `Config` field. A tap runs [`apply`], and [`draw_row`]
//! repaints that row.
//!
//! Blocking sub-loop: [`Layout::row_at`] for geometry, [`render`] for the
//! panel, [`run`] owning input until [`Layout::done`]. GC16 on open and
//! rotate, a single-row DU on a change.
//!
//! `converter` rides every function that draws: the two conversion rows carry
//! the install lines for `crate::convert`'s add-on while it is missing, and
//! [`row_boxes`] gives them the room those lines need.

use std::path::{Path, PathBuf};

use crate::config::{self, Config};
use crate::convert;
use crate::eink::fb::{Framebuffer, MxcfbRect, WAVEFORM_MODE_DU, WAVEFORM_MODE_GC16};
use crate::eink::input::{Input, InputEvent};
use crate::eink::touch::TouchEvent;
use crate::orientation::Orientation;
use crate::ui::text::TextRenderer;

/// Bottom strip height, matching `setup::BTN_H`.
const STRIP_H: u32 = 120;
const MARGIN_X: u32 = 60;
/// Right inset aligning [`Row::value`] into a column.
const VALUE_MARGIN_X: u32 = 60;

/// The rows in display order: what is scanned, what is listed out of it,
/// where it goes, what else is written there, how the grid marks a finished
/// book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    ScanItems01,
    ScanDocuments,
    TypesKfx,
    TypesMobi,
    OutDir,
    PackKfx,
    ConvertEpub,
    ShowDone,
}

pub const ROWS: [Row; 8] = [
    Row::ScanItems01,
    Row::ScanDocuments,
    Row::TypesKfx,
    Row::TypesMobi,
    Row::OutDir,
    Row::PackKfx,
    Row::ConvertEpub,
    Row::ShowDone,
];

impl Row {
    /// Left-aligned name.
    fn label(self) -> &'static str {
        match self {
            Row::ScanItems01 => "Scan purchases",
            Row::ScanDocuments => "Scan library folder",
            Row::TypesKfx => "List KFX books",
            Row::TypesMobi => "List MOBI and AZW3 books",
            Row::OutDir => "Decrypt into",
            Row::PackKfx => "Also pack as KFX",
            Row::ConvertEpub => "Also convert to EPUB",
            Row::ShowDone => "Keep finished books listed",
        }
    }

    /// The second line under a row: the directory a scan row covers, or, while
    /// the add-on is missing, one half of how to install it.
    ///
    /// The two conversion rows are adjacent, so their halves read as the two
    /// steps they are — where to get it, where to put it. Drawn unwrapped:
    /// a wrap would split the URL and the path a reader has to transcribe,
    /// which is what [`DETAIL_MAX_CHARS`] bounds.
    fn detail(self, converter: bool) -> Option<String> {
        match self {
            Row::ScanItems01 => Some(config::ITEMS01_DIR.to_string()),
            Row::ScanDocuments => Some(config::DOCUMENTS_DIR.to_string()),
            Row::PackKfx if !converter => Some(format!("Needs bokai — {}", convert::RELEASES_URL)),
            Row::ConvertEpub if !converter => Some(format!(
                "Unzip {} into {}/",
                convert::RELEASE_ASSET,
                convert::EXTENSION_DIR
            )),
            _ => None,
        }
    }

    /// Right-aligned value. ASCII marks: the firmware's font set carries no
    /// guaranteed box-drawing glyphs.
    fn value(self, cfg: &Config) -> String {
        let check = |on: bool| if on { "[x]" } else { "[ ]" }.to_string();
        match self {
            Row::ScanItems01 => check(cfg.scan_items01),
            Row::ScanDocuments => check(cfg.scan_documents),
            Row::TypesKfx => check(cfg.types_kfx),
            Row::TypesMobi => check(cfg.types_mobi),
            Row::OutDir => cfg.out_dir.display().to_string(),
            Row::PackKfx => check(cfg.pack_kfx),
            Row::ConvertEpub => check(cfg.convert_epub),
            Row::ShowDone => check(cfg.show_done),
        }
    }
}

/// `config::OUT_DIR_PRESETS` plus the `out_dir` [`OutDirs::new`] received.
///
/// [`run`] builds this once and holds it. A list rebuilt per tap drops a
/// hand-written path as soon as [`OutDirs::advance`] steps off it.
#[derive(Debug, Clone)]
pub struct OutDirs {
    options: Vec<PathBuf>,
    index: usize,
}

impl OutDirs {
    /// `config::OUT_DIR_PRESETS`, plus `current` when it is not among them.
    pub fn new(current: &Path) -> Self {
        let mut options: Vec<PathBuf> = config::OUT_DIR_PRESETS.iter().map(PathBuf::from).collect();
        if !options.iter().any(|p| p == current) {
            options.push(current.to_path_buf());
        }
        let index = options.iter().position(|p| p == current).unwrap_or(0);
        OutDirs { options, index }
    }

    /// The next entry, wrapping.
    pub fn advance(&mut self) -> PathBuf {
        self.index = (self.index + 1) % self.options.len();
        self.options[self.index].clone()
    }
}

/// Least height a row may take, whatever its text measures. A comfortable
/// finger target on a ~300 DPI panel.
const MIN_ROW_H: u32 = 76;

/// Chars a [`Row::detail`] may carry before it runs off the panel.
///
/// `pager::NARROWEST_PANEL_W` less both margins, over the ~half-em advance a
/// proportional face averages at `app::FONT_PX`. `TextRenderer::measure_width`
/// is the exact answer and needs a font this crate's tests do not have.
const DETAIL_MAX_CHARS: usize = 59;

/// One row's slice of the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RowBox {
    top: u32,
    height: u32,
    /// Whether [`Row::detail`]'s line fits under the label here.
    detail: bool,
}

/// Where the rows sit between `rows_top` and the strip `room` px below it.
///
/// A row carrying a [`Row::detail`] is a line taller than one that does not —
/// until eight of them stop fitting, which a panel held in landscape is short
/// enough to do. The detail lines go first then, and the air between rows
/// after them: a row past the strip is untappable, [`Layout::row_at`]
/// returning `None` below it.
///
/// Free of the framebuffer and the font, so the test that checks the eight
/// rows clear the strip measures the layout itself.
fn row_boxes(rows_top: u32, lh: u32, room: u32, converter: bool) -> Vec<RowBox> {
    let plan = |details: bool, air: u32| -> Vec<RowBox> {
        let mut y = rows_top;
        ROWS.iter()
            .map(|row| {
                let detail = details && row.detail(converter).is_some();
                // [`Row::label`], the [`Row::detail`] under it, then the air.
                let height = (lh * if detail { 2 } else { 1 } + air).max(MIN_ROW_H);
                let top = y;
                y += height;
                RowBox {
                    top,
                    height,
                    detail,
                }
            })
            .collect()
    };
    let fits = |boxes: &[RowBox]| {
        boxes
            .last()
            .is_some_and(|b| b.top + b.height - rows_top <= room)
    };

    let full = plan(true, lh / 2);
    if fits(&full) {
        return full;
    }
    let tight = plan(false, lh / 2);
    if fits(&tight) {
        return tight;
    }
    plan(false, 0)
}

struct Layout {
    rows_top: u32,
    rows: Vec<RowBox>,
    strip_top: u32,
}

impl Layout {
    fn compute(renderer: &TextRenderer, yres: u32, converter: bool) -> Self {
        let lh = renderer.line_height().max(1);
        let rows_top = lh * 3;
        let strip_top = yres.saturating_sub(STRIP_H);
        Layout {
            rows_top,
            rows: row_boxes(rows_top, lh, strip_top.saturating_sub(rows_top), converter),
            strip_top,
        }
    }

    /// The [`ROWS`] index at `ty`; `None` over the title band and the strip.
    fn row_at(&self, ty: u32) -> Option<usize> {
        if ty < self.rows_top || ty >= self.strip_top {
            return None;
        }
        self.rows
            .iter()
            .position(|b| ty >= b.top && ty < b.top + b.height)
    }

    fn done(&self, ty: u32) -> bool {
        ty >= self.strip_top
    }

    fn row_rect(&self, slot: usize, xres: u32) -> MxcfbRect {
        MxcfbRect {
            top: self.rows[slot].top,
            left: 0,
            width: xres,
            height: self.rows[slot].height,
        }
    }
}

fn full_rect(fb: &Framebuffer) -> MxcfbRect {
    MxcfbRect {
        top: 0,
        left: 0,
        width: fb.var.xres,
        height: fb.var.yres,
    }
}

/// One tap on `row`. [`Row::OutDir`] reads `out_dirs`, which carries state
/// `cfg` does not.
fn apply(row: Row, cfg: &mut Config, out_dirs: &mut OutDirs) {
    match row {
        Row::ScanItems01 => cfg.scan_items01 = !cfg.scan_items01,
        Row::ScanDocuments => cfg.scan_documents = !cfg.scan_documents,
        Row::TypesKfx => cfg.types_kfx = !cfg.types_kfx,
        Row::TypesMobi => cfg.types_mobi = !cfg.types_mobi,
        Row::PackKfx => cfg.pack_kfx = !cfg.pack_kfx,
        Row::ConvertEpub => cfg.convert_epub = !cfg.convert_epub,
        Row::ShowDone => cfg.show_done = !cfg.show_done,
        Row::OutDir => cfg.out_dir = out_dirs.advance(),
    }
}

fn draw_row(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    layout: &Layout,
    slot: usize,
    cfg: &Config,
    converter: bool,
) {
    let row = ROWS[slot];
    let xres = fb.var.xres;
    let RowBox {
        top,
        height,
        detail,
    } = layout.rows[slot];
    fb.fill_rect(top, 0, xres, height, 0xFF);

    let lh = renderer.line_height().max(1);
    let baseline = (top + lh * 80 / 100) as i32;
    renderer.draw(fb, MARGIN_X as i32, baseline, row.label(), false);

    // Right-aligned across labels of differing width.
    let value = row.value(cfg);
    let vw = renderer.measure_width(&value);
    let vx = (xres.saturating_sub(VALUE_MARGIN_X) as i32 - vw as i32).max(MARGIN_X as i32);
    renderer.draw(fb, vx, baseline, &value, false);

    // Dropped when [`row_boxes`] could not fit eight two-line rows.
    if let Some(text) = row.detail(converter).filter(|_| detail) {
        renderer.draw(
            fb,
            MARGIN_X as i32,
            (top + lh * 180 / 100) as i32,
            &text,
            false,
        );
    }
}

fn render(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    cfg: &Config,
    layout: &Layout,
    converter: bool,
) {
    let xres = fb.var.xres;
    fb.fill_rect(0, 0, xres, fb.var.yres, 0xFF);

    let title = "Settings";
    let tw = renderer.measure_width(title);
    let tx = ((xres as i32 - tw as i32) / 2).max(0);
    renderer.draw(fb, tx, (layout.rows_top - 8) as i32, title, false);

    for slot in 0..ROWS.len() {
        draw_row(fb, renderer, layout, slot, cfg, converter);
    }

    // `Config::lists_anything` is false two taps from the default.
    if !cfg.lists_anything() {
        let warn = "Nothing will be listed — turn on a folder and a format.";
        let ww = renderer.measure_width(warn);
        let wx = ((xres as i32 - ww as i32) / 2).max(0);
        let lh = renderer.line_height().max(1);
        renderer.draw(fb, wx, (layout.strip_top - lh) as i32, warn, false);
    }

    draw_strip(fb, renderer, layout);
}

fn draw_strip(fb: &mut Framebuffer, renderer: &mut TextRenderer, layout: &Layout) {
    let xres = fb.var.xres;
    let top = layout.strip_top;
    fb.fill_rect(top, 0, xres, 2, 0x00);
    fb.fill_rect(top + 2, 0, xres, STRIP_H - 2, 0xFF);

    let label = "[ Done ]";
    let w = renderer.measure_width(label);
    let x = ((xres as i32 - w as i32) / 2).max(0);
    renderer.draw(fb, x, (top + STRIP_H * 60 / 100) as i32, label, false);
}

/// Owns input until [`Layout::done`], mutating `cfg` in place.
///
/// `converter` is whether `convert::locate` found the add-on. The two
/// conversion rows toggle either way — the setting outlives an install — and
/// carry the install line under them while it is `false`.
pub fn run(
    fb: &mut Framebuffer,
    input: &mut Input,
    renderer: &mut TextRenderer,
    cfg: &mut Config,
    orient: &mut Orientation,
    converter: bool,
) -> anyhow::Result<()> {
    let mut layout = Layout::compute(renderer, fb.var.yres, converter);
    let mut out_dirs = OutDirs::new(&cfg.out_dir);

    render(fb, renderer, cfg, &layout, converter);
    fb.send_update(full_rect(fb), WAVEFORM_MODE_GC16)?;

    loop {
        match input.next_event()? {
            InputEvent::Touch(TouchEvent::Up { x: _, y }) => {
                if layout.done(y) {
                    return Ok(());
                }
                let Some(slot) = layout.row_at(y) else {
                    continue;
                };
                let listed_before = cfg.lists_anything();
                apply(ROWS[slot], cfg, &mut out_dirs);
                if cfg.lists_anything() == listed_before {
                    // One row changed: DU that row.
                    draw_row(fb, renderer, &layout, slot, cfg, converter);
                    fb.send_update(layout.row_rect(slot, fb.var.xres), WAVEFORM_MODE_DU)?;
                } else {
                    // The `lists_anything` line sits outside this row's rect.
                    render(fb, renderer, cfg, &layout, converter);
                    fb.send_update(full_rect(fb), WAVEFORM_MODE_GC16)?;
                }
            }
            InputEvent::Touch(TouchEvent::Screenshot) => {
                let _ = crate::eink::screenshot::capture(fb);
            }
            InputEvent::Tick => {
                let o = Orientation::detect();
                if o != *orient {
                    *orient = o;
                    input.set_orientation(o);
                    layout = Layout::compute(renderer, fb.var.yres, converter);
                    render(fb, renderer, cfg, &layout, converter);
                    fb.send_update(full_rect(fb), WAVEFORM_MODE_GC16)?;
                }
            }
            // `Input` holds the bezel keys grabbed; a press reaches no further.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cycle_from(start: &str) -> Vec<String> {
        let mut c = OutDirs::new(Path::new(start));
        // One full lap plus one, covering the wrap.
        (0..=config::OUT_DIR_PRESETS.len() + 1)
            .map(|_| c.advance().display().to_string())
            .collect()
    }

    #[test]
    fn stepping_through_the_presets_wraps() {
        let seen = cycle_from(config::DEFAULT_OUT_DIR);
        for preset in config::OUT_DIR_PRESETS {
            assert!(seen.contains(&preset.to_string()), "{preset} unreachable");
        }
        // The lap returns to `config::DEFAULT_OUT_DIR`.
        assert!(seen.contains(&config::DEFAULT_OUT_DIR.to_string()));
    }

    #[test]
    fn a_hand_written_path_survives_a_full_lap() {
        // A path outside `config::OUT_DIR_PRESETS` stays a stop across a lap.
        let custom = "/mnt/us/my books/dedrm";
        let seen = cycle_from(custom);
        assert!(
            seen.contains(&custom.to_string()),
            "custom path lost after a lap: {seen:?}"
        );
        assert_eq!(seen.len(), config::OUT_DIR_PRESETS.len() + 2);
    }

    #[test]
    fn every_toggle_row_flips_exactly_its_own_setting() {
        for row in ROWS {
            if row == Row::OutDir {
                continue;
            }
            let before = Config::default();
            let mut after = before.clone();
            let mut dirs = OutDirs::new(&after.out_dir);
            apply(row, &mut after, &mut dirs);
            assert_ne!(before, after, "{row:?} changed nothing");
            // `out_dir` is not a toggle.
            assert_eq!(before.out_dir, after.out_dir, "{row:?} moved the out dir");
            // Flipping twice is the identity.
            apply(row, &mut after, &mut dirs);
            assert_eq!(before, after, "{row:?} did not flip back");
        }
    }

    #[test]
    fn the_out_dir_row_moves_only_the_out_dir() {
        let before = Config::default();
        let mut after = before.clone();
        let mut dirs = OutDirs::new(&after.out_dir);
        apply(Row::OutDir, &mut after, &mut dirs);
        assert_ne!(before.out_dir, after.out_dir);
        assert_eq!(
            Config {
                out_dir: before.out_dir.clone(),
                ..after.clone()
            },
            before
        );
    }

    #[test]
    fn every_row_renders_a_label_and_a_value() {
        let cfg = Config::default();
        for row in ROWS {
            assert!(!row.label().is_empty(), "{row:?}");
            assert!(!row.value(&cfg).is_empty(), "{row:?}");
        }
        // The scan rows carry a [`Row::detail`] whatever else holds.
        for converter in [true, false] {
            assert_eq!(
                Row::ScanItems01.detail(converter).as_deref(),
                Some(config::ITEMS01_DIR)
            );
            assert_eq!(
                Row::ScanDocuments.detail(converter).as_deref(),
                Some(config::DOCUMENTS_DIR)
            );
            assert_eq!(Row::ShowDone.detail(converter), None);
        }
    }

    #[test]
    fn the_conversion_rows_name_the_asset_the_url_and_the_destination() {
        // Between them, and only while the add-on is missing.
        let text = [Row::PackKfx, Row::ConvertEpub]
            .map(|row| row.detail(false).expect("no install line"))
            .join("\n");
        assert!(text.contains(convert::RELEASES_URL));
        assert!(text.contains(convert::RELEASE_ASSET));
        assert!(text.contains(convert::EXTENSION_DIR));
        // Installed, the lines are gone and both rows are a line shorter.
        assert_eq!(Row::PackKfx.detail(true), None);
        assert_eq!(Row::ConvertEpub.detail(true), None);
    }

    #[test]
    fn no_detail_line_runs_off_the_narrowest_panel() {
        for row in ROWS {
            for converter in [true, false] {
                let Some(detail) = row.detail(converter) else {
                    continue;
                };
                let n = detail.chars().count();
                assert!(
                    n <= DETAIL_MAX_CHARS,
                    "{row:?} detail is {n} chars, over {DETAIL_MAX_CHARS}: {detail}"
                );
            }
        }
    }

    /// The panel is at its shortest held in landscape on the narrowest device
    /// this runs on, and every row has to stay above the `[ Done ]` strip
    /// there — [`Layout::row_at`] returns `None` below it.
    #[test]
    fn every_row_stays_tappable_on_the_shortest_panel() {
        // `pager::NARROWEST_PANEL_W` is that panel's short side, so it is the
        // `yres` a landscape rotation hands this layout.
        let yres = crate::ui::pager::NARROWEST_PANEL_W;
        // Past anything `app::FONT_PX` produces at either end, standing in for
        // whichever face the fallback chain resolves to.
        for lh in 24..=64 {
            let rows_top = lh * 3;
            let room = yres - STRIP_H - rows_top;
            for converter in [true, false] {
                let boxes = row_boxes(rows_top, lh, room, converter);
                assert_eq!(boxes.len(), ROWS.len());
                let last = boxes[ROWS.len() - 1];
                assert!(
                    last.top + last.height <= yres - STRIP_H,
                    "rows run to {}px past {}px of room (lh={lh} converter={converter})",
                    last.top + last.height - rows_top,
                    room
                );
                // Contiguous and ascending, so [`Layout::row_at`]'s scan finds
                // a row for every y the strip does not take.
                assert_eq!(boxes[0].top, rows_top);
                assert!(
                    boxes.windows(2).all(|w| w[0].top + w[0].height == w[1].top),
                    "gap between rows (lh={lh})"
                );
                // Every row keeps a finger-sized target through both fallbacks.
                assert!(boxes.iter().all(|b| b.height >= MIN_ROW_H));
            }
        }
    }

    #[test]
    fn a_row_is_taller_exactly_when_it_carries_a_detail_line() {
        const LH: u32 = 40;
        // Room enough that [`row_boxes`] keeps every detail line.
        let boxes = row_boxes(0, LH, 4000, true);
        let at = |row: Row| boxes[ROWS.iter().position(|r| *r == row).unwrap()];
        assert!(at(Row::ScanItems01).detail);
        assert!(!at(Row::ShowDone).detail);
        assert!(at(Row::ScanItems01).height > at(Row::ShowDone).height);
    }

    #[test]
    fn a_panel_too_short_for_the_detail_lines_drops_them_rather_than_the_rows() {
        const LH: u32 = 40;
        // Exactly the eight one-line rows, and not one pixel more.
        let boxes = row_boxes(0, LH, MIN_ROW_H * ROWS.len() as u32, false);
        assert!(boxes.iter().all(|b| !b.detail));
        assert!(boxes.iter().all(|b| b.height == MIN_ROW_H));
    }
}
