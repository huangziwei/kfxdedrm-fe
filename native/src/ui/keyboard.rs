//! On-screen keyboard typing `Config::out_dir`: a QWERTY grid with `Del` at
//! the number row's right end, `[ Done ]` far right and `[ Back ]` leftmost. A
//! blocking sub-loop shaped like [`crate::ui::configmenu`]; a single-band DU on
//! a keystroke.
//!
//! [`ROWS`] carries the punctuation a path is spelled with — `/` and `.` at the
//! end of the bottom row, `-` at the end of the number row — and [`Key::Shift`]
//! reaches the upper case a folder created off the device may carry.

use std::path::{Path, PathBuf};

use crate::config;
use crate::eink::fb::{Framebuffer, MxcfbRect, WAVEFORM_MODE_DU, WAVEFORM_MODE_GC16};
use crate::eink::input::{Input, InputEvent};
use crate::eink::touch::TouchEvent;
use crate::orientation::Orientation;
use crate::ui::grid::outline_rect;
use crate::ui::pathbar;
use crate::ui::text::TextRenderer;

/// Letter/digit rows, following a physical keyboard. Row 0 carries `Del` in a
/// twelfth cell at its right end, appended by [`layout`]; row 3 ends in the
/// three marks a path is spelled with.
const ROWS: [&str; 4] = ["1234567890-", "qwertyuiop", "asdfghjkl", "zxcvbnm,./"];

/// Cells in row 0, [`ROWS`] plus `Del`.
const DIGIT_CELLS: i32 = 12;

/// Gap between key faces, and the panel margin.
const GAP: i32 = 8;
const MARGIN: i32 = 20;

/// Bottom command strip: `[ Back ] | Shift | space | Clear | [ Done ]`. Height
/// matches `ui/panel.rs`, `ZONE_W` the fixed slots in `ui/pager.rs`, landing
/// `[ Back ]` where `Exit` sits.
const STRIP_H: u32 = 120;
const ZONE_W: u32 = 200;
/// Slots in the strip, for the width [`layout`] leaves `space`.
const ZONES: u32 = 5;
/// Thickness of the strip's divider rules, matching the other bottom strips.
const RULE: u32 = 2;

#[derive(Clone, Copy, PartialEq)]
enum Key {
    Char(char),
    Space,
    Backspace,
    Clear,
    /// Upper case for the next [`Key::Char`], and `_` for `-`. Cleared by the
    /// keystroke it applies to.
    Shift,
    /// Leave without writing, returning the path the overlay opened with.
    /// Distinct from [`Key::Done`], this keyboard's Enter.
    Back,
    /// Submit the typed path. Inert while [`config::out_dir_refusal`] names a
    /// reason.
    Done,
}

/// `c` under [`Key::Shift`].
fn shifted(c: char) -> char {
    match c {
        '-' => '_',
        c => c.to_ascii_uppercase(),
    }
}

/// How a key is drawn. Both kinds hit-test the same way.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Style {
    /// A key in the grid: an outlined face inset from its cell.
    Face,
    /// A slot in the bottom strip: a label only, with the strip drawing the
    /// dividers around it.
    Zone,
}

/// `x`/`y`/`w`/`h` is the **cell**, the whole tappable area. A `Face` key draws
/// inset by half a gap, leaving the gutters inside a cell.
struct KeyButton {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    key: Key,
    label: String,
    style: Style,
}

/// Bottom of the top band: the path field and the verdict line under it. A
/// keystroke refreshes only `[0, band_bottom]`, leaving the keyboard untouched.
fn band_bottom(lh: u32) -> u32 {
    pathbar::TOP + pathbar::HEIGHT + lh + 24
}

fn strip_top(yres: u32) -> u32 {
    yres.saturating_sub(STRIP_H)
}

/// Keyboard metrics `(unit, unit_digits, key_h, keys_top)`. Letter rows divide
/// the span into ten columns, the digit row into [`DIGIT_CELLS`]. Key faces are
/// square, and the four-row block anchors above the command strip.
fn metrics(xres: u32, yres: u32) -> (i32, i32, i32, i32) {
    let span = (xres as i32 - 2 * MARGIN).max(1);
    let unit = (span / 10).max(1);
    let unit_digits = (span / DIGIT_CELLS).max(1);
    let key_h = (unit - GAP).max(1);
    let block_h = 4 * key_h + 3 * GAP;
    let keys_top = (yres as i32 - STRIP_H as i32 - MARGIN - block_h).max(MARGIN);
    (unit, unit_digits, key_h, keys_top)
}

/// Lay out every key: the four letter/digit rows, then the five strip slots.
fn layout(xres: u32, yres: u32) -> Vec<KeyButton> {
    let (unit, unit_digits, key_h, top) = metrics(xres, yres);
    let stride = key_h + GAP;
    let mut out = Vec::new();

    for (r, row) in ROWS.iter().enumerate() {
        let digits = r == 0;
        let u = if digits { unit_digits } else { unit };
        // The digit row reserves one extra column for `Del`.
        let n = row.chars().count() as i32 + i32::from(digits);
        let start_x = (xres as i32 - n * u) / 2;
        let y = top + r as i32 * stride;
        for (i, c) in row.chars().enumerate() {
            out.push(KeyButton {
                x: start_x + i as i32 * u,
                y,
                w: u as u32,
                h: stride as u32,
                key: Key::Char(c),
                label: c.to_string(),
                style: Style::Face,
            });
        }
        if digits {
            out.push(KeyButton {
                x: start_x + (n - 1) * u,
                y,
                w: u as u32,
                h: stride as u32,
                key: Key::Backspace,
                label: "Del".to_string(),
                style: Style::Face,
            });
        }
    }

    // `[ Back ]` takes the leftmost slot, the one `crate::ui::pager` gives
    // `Exit`. `[ Done ]` sits far right, the wide `space` in the middle.
    let sy = strip_top(yres) as i32;
    let side = ZONE_W.min(xres / ZONES);
    for (x, w, key, label) in [
        (0, side, Key::Back, "[ Back ]"),
        (side as i32, side, Key::Shift, "Shift"),
        (
            (side * 2) as i32,
            xres.saturating_sub(side * 4),
            Key::Space,
            "space",
        ),
        (
            xres.saturating_sub(side * 2) as i32,
            side,
            Key::Clear,
            "Clear",
        ),
        (
            xres.saturating_sub(side) as i32,
            side,
            Key::Done,
            "[ Done ]",
        ),
    ] {
        out.push(KeyButton {
            x,
            y: sy,
            w,
            h: STRIP_H,
            key,
            label: label.to_string(),
            style: Style::Zone,
        });
    }
    out
}

fn full_rect(fb: &Framebuffer) -> MxcfbRect {
    MxcfbRect {
        top: 0,
        left: 0,
        width: fb.var.xres,
        height: fb.var.yres,
    }
}

/// The path band at the top, its own rect for a per-keystroke DU.
fn band_rect(fb: &Framebuffer, lh: u32) -> MxcfbRect {
    MxcfbRect {
        top: 0,
        left: 0,
        width: fb.var.xres,
        height: band_bottom(lh),
    }
}

/// The drawn face of a key: a grid cell inset by half a gap, a strip slot inset
/// past the strip's top and left rules. Those rules are drawn inside the slot
/// rects, and a full-cell fill paints over them.
fn face(kb: &KeyButton) -> (i32, i32, u32, u32) {
    match kb.style {
        Style::Zone => (
            kb.x + RULE as i32,
            kb.y + RULE as i32,
            kb.w.saturating_sub(RULE).max(1),
            kb.h.saturating_sub(RULE).max(1),
        ),
        Style::Face => {
            let inset = GAP / 2;
            (
                kb.x + inset,
                kb.y + inset,
                kb.w.saturating_sub(GAP as u32).max(1),
                kb.h.saturating_sub(GAP as u32).max(1),
            )
        }
    }
}

fn key_rect(kb: &KeyButton) -> MxcfbRect {
    let (x, y, w, h) = face(kb);
    MxcfbRect {
        top: y.max(0) as u32,
        left: x.max(0) as u32,
        width: w,
        height: h,
    }
}

/// What `[ Done ]` does with `path`, as the line under the field.
///
/// `config::out_dir_refusal` is the check `Config::sanitized` applies to a
/// hand-edited file. A path this line accepts survives being written to one.
fn verdict(path: &str, scan_dirs: &[PathBuf]) -> String {
    let dir = Path::new(path);
    match config::out_dir_refusal(dir, scan_dirs) {
        Some(reason) => format!("Not here — {reason}"),
        None if dir.is_dir() => "This folder exists".to_string(),
        None => "Tap Done to create it".to_string(),
    }
}

/// [`crate::ui::pathbar`] over the [`verdict`] line. The caller white-fills the
/// band first.
///
/// Both draw in black: [`band_rect`] refreshes under `WAVEFORM_MODE_DU`, which
/// is two-level and snaps `ui::QUIET`.
fn draw_band(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    path: &str,
    scan_dirs: &[PathBuf],
    lh: u32,
) {
    let xres = fb.var.xres;
    pathbar::draw(fb, renderer, path);

    let line = verdict(path, scan_dirs);
    let lw = renderer.measure_width(&line);
    let ly = (pathbar::TOP + pathbar::HEIGHT + lh) as i32;
    renderer.draw(fb, ((xres as i32 - lw as i32) / 2).max(0), ly, &line, false);
}

/// One key. `pressed` inverts it, filled black with a white label: the whole
/// acknowledgement a tap gets under the finger, ahead of the band refresh at
/// the far end of the screen.
fn draw_key(fb: &mut Framebuffer, renderer: &mut TextRenderer, kb: &KeyButton, pressed: bool) {
    let (x, y, w, h) = face(kb);
    let (top, left) = (y.max(0) as u32, x.max(0) as u32);
    fb.fill_rect(top, left, w, h, if pressed { 0x00 } else { 0xFF });
    if !pressed && kb.style == Style::Face {
        outline_rect(fb, x, y, w, h, 2, 0x00);
    }
    let lw = renderer.measure_width(&kb.label);
    let tx = x + ((w as i32 - lw as i32) / 2).max(0);
    let baseline = y + (h * 62 / 100) as i32;
    renderer.draw(fb, tx, baseline, &kb.label, pressed);
}

/// The strip's chrome: the rule above it and the slot separators, drawn the same
/// way `ui/pager.rs` draws its own. [`face`] insets a slot past these, leaving
/// them intact under a press.
fn draw_strip_chrome(fb: &mut Framebuffer, keys: &[KeyButton]) {
    let xres = fb.var.xres;
    let top = strip_top(fb.var.yres);
    fb.fill_rect(top, 0, xres, RULE, 0x00);
    for kb in keys.iter().filter(|k| k.style == Style::Zone) {
        if kb.x > 0 {
            fb.fill_rect(top + 12, kb.x.max(0) as u32, RULE, STRIP_H - 24, 0x00);
        }
    }
}

/// The [`Key::Shift`] slot, drawn inverted while the latch is set.
fn shift_index(keys: &[KeyButton]) -> Option<usize> {
    keys.iter().position(|k| k.key == Key::Shift)
}

fn render_all(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    keys: &[KeyButton],
    path: &str,
    scan_dirs: &[PathBuf],
    shift: bool,
    lh: u32,
) {
    fb.fill_rect(0, 0, fb.var.xres, fb.var.yres, 0xFF);
    draw_band(fb, renderer, path, scan_dirs, lh);
    let latched = shift.then(|| shift_index(keys)).flatten();
    for (i, kb) in keys.iter().enumerate() {
        draw_key(fb, renderer, kb, Some(i) == latched);
    }
    draw_strip_chrome(fb, keys);
}

/// Index of the key under a touch. Cells tile their row, landing a gutter tap
/// on a neighbouring key.
fn hit_index(keys: &[KeyButton], tx: u32, ty: u32) -> Option<usize> {
    let (tx, ty) = (tx as i32, ty as i32);
    keys.iter()
        .position(|k| tx >= k.x && tx < k.x + k.w as i32 && ty >= k.y && ty < k.y + k.h as i32)
}

fn hit(keys: &[KeyButton], tx: u32, ty: u32) -> Option<Key> {
    hit_index(keys, tx, ty).map(|i| keys[i].key)
}

/// Runs the keyboard, returning the typed path on `[ Done ]` and `initial` on
/// `[ Back ]`. The caller acts on a difference.
///
/// `initial` pre-fills the field. `scan_dirs` is what [`verdict`] reads a
/// collision against; `[ Done ]` returns nothing while one stands.
pub fn run(
    fb: &mut Framebuffer,
    input: &mut Input,
    renderer: &mut TextRenderer,
    initial: &Path,
    scan_dirs: &[PathBuf],
    orient: &mut Orientation,
) -> anyhow::Result<PathBuf> {
    let lh = renderer.line_height().max(1);
    let mut path = initial.display().to_string();
    let mut keys = layout(fb.var.xres, fb.var.yres);
    // The key currently held down, un-inverted on release.
    let mut pressed: Option<usize> = None;
    let mut shift = false;

    render_all(fb, renderer, &keys, &path, scan_dirs, shift, lh);
    fb.send_update(full_rect(fb), WAVEFORM_MODE_GC16)?;

    // Refresh just the path+verdict band after a keystroke (fast DU, no flash).
    macro_rules! refresh_band {
        () => {{
            fb.fill_rect(0, 0, fb.var.xres, band_bottom(lh), 0xFF);
            draw_band(fb, renderer, &path, scan_dirs, lh);
            fb.send_update(band_rect(fb, lh), WAVEFORM_MODE_DU)?;
        }};
    }

    // The latch is drawn on its own slot, outside the band.
    macro_rules! refresh_shift {
        () => {{
            if let Some(i) = shift_index(&keys) {
                draw_key(fb, renderer, &keys[i], shift);
                fb.send_update(key_rect(&keys[i]), WAVEFORM_MODE_DU)?;
            }
        }};
    }

    loop {
        match input.next_event()? {
            InputEvent::Touch(TouchEvent::Up { x, y }) => {
                if let Some(i) = pressed.take() {
                    let latched = shift && keys[i].key == Key::Shift;
                    draw_key(fb, renderer, &keys[i], latched);
                    fb.send_update(key_rect(&keys[i]), WAVEFORM_MODE_DU)?;
                }
                // `pathbar`'s `✕` clears. Past it, a key resolves.
                if pathbar::clear_hit(x, y, fb.var.xres, !path.is_empty()) {
                    path.clear();
                    refresh_band!();
                } else {
                    match hit(&keys, x, y) {
                        Some(Key::Char(c)) => {
                            path.push(if shift { shifted(c) } else { c });
                            if shift {
                                shift = false;
                                refresh_shift!();
                            }
                            refresh_band!();
                        }
                        Some(Key::Space) => {
                            path.push(' ');
                            refresh_band!();
                        }
                        Some(Key::Backspace) => {
                            path.pop();
                            refresh_band!();
                        }
                        Some(Key::Clear) => {
                            path.clear();
                            refresh_band!();
                        }
                        Some(Key::Shift) => {
                            shift = !shift;
                            refresh_shift!();
                        }
                        // A refused path redraws the line naming the reason.
                        Some(Key::Done) => {
                            if config::out_dir_refusal(Path::new(&path), scan_dirs).is_none() {
                                return Ok(PathBuf::from(path));
                            }
                            refresh_band!();
                        }
                        // Hand back what the overlay opened with. The caller
                        // compares it against the current folder.
                        Some(Key::Back) => return Ok(initial.to_path_buf()),
                        None => {}
                    }
                }
            }
            InputEvent::Touch(TouchEvent::Down { x, y }) => {
                if !pathbar::clear_hit(x, y, fb.var.xres, !path.is_empty())
                    && let Some(i) = hit_index(&keys, x, y)
                {
                    draw_key(fb, renderer, &keys[i], true);
                    fb.send_update(key_rect(&keys[i]), WAVEFORM_MODE_DU)?;
                    pressed = Some(i);
                }
            }
            InputEvent::Touch(TouchEvent::Screenshot) => {
                let _ = crate::eink::screenshot::capture(fb);
            }
            InputEvent::Page(_) => {}
            InputEvent::Tick => {
                let o = Orientation::detect();
                if o != *orient {
                    *orient = o;
                    input.set_orientation(o);
                    keys = layout(fb.var.xres, fb.var.yres);
                    pressed = None;
                    render_all(fb, renderer, &keys, &path, scan_dirs, shift, lh);
                    fb.send_update(full_rect(fb), WAVEFORM_MODE_GC16)?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XRES: u32 = 1264;
    const YRES: u32 = 1680;

    fn find(keys: &[KeyButton], c: char) -> &KeyButton {
        keys.iter()
            .find(|k| matches!(k.key, Key::Char(x) if x == c))
            .expect("key present")
    }

    #[test]
    fn layout_covers_every_letter_digit_and_path_mark() {
        let keys = layout(XRES, YRES);
        // 12 (digits, `-` and Del) + 10 + 9 + 10 faces, plus five strip slots.
        assert_eq!(keys.len(), 41 + ZONES as usize);
        for c in "abcdefghijklmnopqrstuvwxyz0123456789-,./".chars() {
            assert!(
                keys.iter().any(|k| matches!(k.key, Key::Char(x) if x == c)),
                "missing key {c}"
            );
        }
    }

    #[test]
    fn every_character_a_kindle_path_is_spelled_with_is_reachable() {
        let keys = layout(XRES, YRES);
        for path in [
            config::DEFAULT_OUT_DIR,
            "/mnt/us/documents/Downloads/Items01",
            "/mnt/us/documents/de-drm_out",
        ] {
            for c in path.chars() {
                let typed = keys.iter().any(|k| match k.key {
                    Key::Char(x) => x == c || shifted(x) == c,
                    Key::Space => c == ' ',
                    _ => false,
                });
                assert!(typed, "{path} needs {c:?}");
            }
        }
    }

    #[test]
    fn del_sits_at_the_right_end_of_the_number_row() {
        let keys = layout(XRES, YRES);
        let dash = find(&keys, '-');
        let del = keys
            .iter()
            .find(|k| matches!(k.key, Key::Backspace))
            .expect("Del present");
        assert_eq!(del.y, dash.y);
        assert!(del.x > dash.x);
    }

    #[test]
    fn shift_reaches_upper_case_and_the_underscore() {
        assert_eq!(shifted('a'), 'A');
        assert_eq!(shifted('-'), '_');
        // A mark that is its own upper case stays put.
        for c in "1/.,".chars() {
            assert_eq!(shifted(c), c, "{c}");
        }
    }

    #[test]
    fn the_verdict_names_what_done_would_do() {
        let scanned = vec![PathBuf::from(config::ITEMS01_DIR)];
        assert_eq!(verdict("dedrm", &scanned), "Not here — needs a full path");
        assert_eq!(
            verdict(config::ITEMS01_DIR, &scanned),
            "Not here — scanned for books"
        );
        // A folder no device has, under a root every device has.
        assert_eq!(
            verdict("/mnt/us/documents/nothing-here", &scanned),
            "Tap Done to create it"
        );
    }

    #[test]
    fn an_existing_folder_reads_as_one() {
        let dir = std::env::temp_dir().join("kfxdedrm-fe-keyboard-verdict");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            verdict(&dir.display().to_string(), &[]),
            "This folder exists"
        );
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn the_strip_slots_tile_the_width_without_a_gap() {
        let keys = layout(XRES, YRES);
        let mut zones: Vec<&KeyButton> = keys.iter().filter(|k| k.style == Style::Zone).collect();
        zones.sort_by_key(|k| k.x);
        assert_eq!(zones.len(), ZONES as usize);
        assert_eq!(zones[0].x, 0);
        for pair in zones.windows(2) {
            assert_eq!(pair[0].x + pair[0].w as i32, pair[1].x);
        }
        let last = zones.last().unwrap();
        assert_eq!(last.x + last.w as i32, XRES as i32);
    }
}
