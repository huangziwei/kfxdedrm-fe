//! One [`Row`] per `Config` field. A tap runs [`apply`], and [`draw_row`]
//! repaints that row.
//!
//! Blocking sub-loop: [`Layout::row_at`] for geometry, [`render`] for the
//! panel, [`run`] owning input until [`Layout::done`]. GC16 on open and
//! rotate, a single-row DU on a change.

use std::path::{Path, PathBuf};

use crate::config::{self, Config};
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
/// where it goes, how the grid marks a finished book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    ScanItems01,
    ScanDocuments,
    TypesKfx,
    TypesMobi,
    OutDir,
    ShowDone,
}

pub const ROWS: [Row; 6] = [
    Row::ScanItems01,
    Row::ScanDocuments,
    Row::TypesKfx,
    Row::TypesMobi,
    Row::OutDir,
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
            Row::ShowDone => "Keep finished books listed",
        }
    }

    /// The directory a scan row covers, on a second line.
    fn detail(self) -> Option<&'static str> {
        match self {
            Row::ScanItems01 => Some(config::ITEMS01_DIR),
            Row::ScanDocuments => Some(config::DOCUMENTS_DIR),
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

struct Layout {
    rows_top: u32,
    row_h: u32,
    strip_top: u32,
}

impl Layout {
    fn compute(renderer: &TextRenderer, yres: u32) -> Self {
        let lh = renderer.line_height().max(1);
        // Holds [`Row::label`] and [`Row::detail`], with a 96px floor.
        let row_h = lh.saturating_mul(2).saturating_add(lh / 2).max(96);
        Layout {
            rows_top: lh * 3,
            row_h,
            strip_top: yres.saturating_sub(STRIP_H),
        }
    }

    /// The [`ROWS`] index at `ty`; `None` over the title band and the strip.
    fn row_at(&self, ty: u32) -> Option<usize> {
        if ty < self.rows_top || ty >= self.strip_top {
            return None;
        }
        let row = ((ty - self.rows_top) / self.row_h) as usize;
        (row < ROWS.len()).then_some(row)
    }

    fn done(&self, ty: u32) -> bool {
        ty >= self.strip_top
    }

    fn row_rect(&self, slot: usize, xres: u32) -> MxcfbRect {
        MxcfbRect {
            top: self.rows_top + slot as u32 * self.row_h,
            left: 0,
            width: xres,
            height: self.row_h,
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
) {
    let row = ROWS[slot];
    let xres = fb.var.xres;
    let top = layout.rows_top + slot as u32 * layout.row_h;
    fb.fill_rect(top, 0, xres, layout.row_h, 0xFF);

    let lh = renderer.line_height().max(1);
    let baseline = (top + lh * 80 / 100) as i32;
    renderer.draw(fb, MARGIN_X as i32, baseline, row.label(), false);

    // Right-aligned across labels of differing width.
    let value = row.value(cfg);
    let vw = renderer.measure_width(&value);
    let vx = (xres.saturating_sub(VALUE_MARGIN_X) as i32 - vw as i32).max(MARGIN_X as i32);
    renderer.draw(fb, vx, baseline, &value, false);

    if let Some(detail) = row.detail() {
        renderer.draw(
            fb,
            MARGIN_X as i32,
            (top + lh * 180 / 100) as i32,
            detail,
            false,
        );
    }
}

fn render(fb: &mut Framebuffer, renderer: &mut TextRenderer, cfg: &Config, layout: &Layout) {
    let xres = fb.var.xres;
    fb.fill_rect(0, 0, xres, fb.var.yres, 0xFF);

    let title = "Settings";
    let tw = renderer.measure_width(title);
    let tx = ((xres as i32 - tw as i32) / 2).max(0);
    renderer.draw(fb, tx, (layout.rows_top - 8) as i32, title, false);

    for slot in 0..ROWS.len() {
        draw_row(fb, renderer, layout, slot, cfg);
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
pub fn run(
    fb: &mut Framebuffer,
    input: &mut Input,
    renderer: &mut TextRenderer,
    cfg: &mut Config,
    orient: &mut Orientation,
) -> anyhow::Result<()> {
    let mut layout = Layout::compute(renderer, fb.var.yres);
    let mut out_dirs = OutDirs::new(&cfg.out_dir);

    render(fb, renderer, cfg, &layout);
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
                    draw_row(fb, renderer, &layout, slot, cfg);
                    fb.send_update(layout.row_rect(slot, fb.var.xres), WAVEFORM_MODE_DU)?;
                } else {
                    // The `lists_anything` line sits outside this row's rect.
                    render(fb, renderer, cfg, &layout);
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
                    layout = Layout::compute(renderer, fb.var.yres);
                    render(fb, renderer, cfg, &layout);
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
        // The scan rows carry a [`Row::detail`]; the rest do not.
        assert_eq!(Row::ScanItems01.detail(), Some(config::ITEMS01_DIR));
        assert_eq!(Row::ScanDocuments.detail(), Some(config::DOCUMENTS_DIR));
        assert_eq!(Row::ShowDone.detail(), None);
    }
}
