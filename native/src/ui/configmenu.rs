//! The Settings page: which [`Row`] each of `ui::panel`'s chips belongs to,
//! and what a tap on it does to [`Config`].
//!
//! [`page`] builds the panel from the settings and [`apply`] is the only thing
//! that writes back. The items and their rows are built in one pass, so a chip
//! cannot be drawn on one row and applied to another.
//!
//! The Scan chips come from `scan::candidates` rather than from a list here:
//! which folder a firmware downloads into has moved before, and a sideload
//! folder is whatever its owner named it.
//!
//! Blocking sub-loop: [`run`] owns input until a tap on the strip. GC16 on
//! open and on rotate, a single-row DU on a change that moves nothing else.

use crate::config::{self, Config};
use crate::convert;
use crate::eink::fb::{Framebuffer, WAVEFORM_MODE_DU, WAVEFORM_MODE_GC16};
use crate::eink::input::{Input, InputEvent};
use crate::eink::touch::TouchEvent;
use crate::orientation::Orientation;
use crate::scan::Candidate;
use crate::ui::panel::{self, Chip, Item, Layout};
use crate::ui::text::TextRenderer;

/// The title's size, against `app::FONT_PX` for everything else.
const TITLE_PX: f32 = 44.0;

/// Chars an [`Item::Note`] line may carry before it runs off the panel.
///
/// `pager::NARROWEST_PANEL_W` less `panel::ROW_INSET` and the right margin,
/// over the ~half-em advance a proportional face averages at `app::FONT_PX`.
/// A note is drawn unwrapped: a wrap would split the URL and the path a reader
/// has to transcribe by hand.
const NOTE_MAX_CHARS: usize = 55;

/// A row of the page whose chips change a setting.
///
/// The page also carries headings and notes; those are not rows in this sense,
/// and [`page`] leaves them out of its row table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// One independent toggle per `scan::Candidate`, in the order given.
    Scan,
    /// One of [`FORMAT_SETS`].
    Formats,
    /// Two independent toggles, inert without the `crate::convert` add-on.
    Convert,
    /// One toggle.
    ShowDone,
}

/// The format sets the panel offers, as `(label, types_kfx, types_mobi)`.
///
/// The file holds the two switches separately and a hand-edited one may say
/// something neither of these does; the row then fills no chip until a tap
/// puts it back on one of them.
const FORMAT_SETS: [(&str, bool, bool); 2] = [("KFX only", true, false), ("All", true, true)];

/// The panel, and which [`Row`] each of its items belongs to.
pub struct Page {
    pub items: Vec<Item>,
    /// One entry per item: the row it changes, or `None` for a heading or a
    /// note.
    rows: Vec<Option<Row>>,
}

impl Page {
    fn push(&mut self, item: Item, row: Option<Row>) {
        self.items.push(item);
        self.rows.push(row);
    }

    /// The [`Row`] item `i` belongs to.
    pub fn row(&self, i: usize) -> Option<Row> {
        self.rows.get(i).copied().flatten()
    }
}

/// The status line under the title.
fn status(converter: bool) -> &'static str {
    if converter {
        "Changes apply as you tap."
    } else {
        // The one thing on this page that has to be done off the device.
        "Changes apply as you tap. Two chips below need an add-on."
    }
}

/// What the Scan row adds up to.
fn scan_note(cfg: &Config, folders: &[Candidate]) -> String {
    if cfg.scan_dirs.is_empty() {
        return "Nothing selected — the grid will be empty".to_string();
    }
    let books: usize = folders
        .iter()
        .filter(|c| cfg.scan_dirs.contains(&c.dir))
        .map(|c| c.books)
        .sum();
    let plural = |n: usize, one: &str| {
        if n == 1 {
            one.to_string()
        } else {
            format!("{one}s")
        }
    };
    let counted = format!("{books} DRM'd {}", plural(books, "book"));
    // "1 of 1 folder" on a device with one is noise; the chip says which.
    if folders.len() < 2 {
        return counted;
    }
    format!(
        "{counted} in {} of {} folders",
        cfg.scan_dirs.len(),
        folders.len()
    )
}

/// Where decrypted books land, or — while the add-on is missing — where to
/// get it.
///
/// Not both: the install is the urgent half, `ui::header` names the
/// destination on every screen the grid draws, and a page has only so many
/// rows before `panel::Layout` cuts one.
fn output_note(converter: bool) -> String {
    if converter {
        return format!("Decrypted books land in {}", config::OUT_DIR);
    }
    format!(
        "Needs bokai — {}\nUnzip {} into {}/",
        convert::RELEASES_URL,
        convert::RELEASE_ASSET,
        convert::EXTENSION_DIR
    )
}

/// The whole page in draw order, from the settings it shows.
///
/// `folders` is `scan::candidates`, which holds every folder already selected
/// even at zero books — a selection with no chip could not be undone.
///
/// `converter` is whether `convert::locate` found the add-on. The two convert
/// chips are drawn either way — the setting outlives an install — but greyed
/// and untappable while it is missing, with the install lines under them.
pub fn page(cfg: &Config, folders: &[Candidate], converter: bool) -> Page {
    let mut p = Page {
        items: Vec::new(),
        rows: Vec::new(),
    };

    p.push(Item::Heading("Books".into()), None);
    p.push(
        Item::Choice {
            label: "Scan".into(),
            chips: folders
                .iter()
                .map(|c| Chip::new(c.label(), cfg.scan_dirs.contains(&c.dir)))
                .collect(),
        },
        Some(Row::Scan),
    );
    p.push(Item::Note(scan_note(cfg, folders)), None);
    p.push(
        Item::Choice {
            label: "Formats".into(),
            chips: FORMAT_SETS
                .iter()
                .map(|(label, kfx, mobi)| {
                    Chip::new(*label, cfg.types_kfx == *kfx && cfg.types_mobi == *mobi)
                })
                .collect(),
        },
        Some(Row::Formats),
    );
    p.push(
        Item::Choice {
            label: "Finished".into(),
            chips: vec![Chip::new("Keep listed", cfg.show_done)],
        },
        Some(Row::ShowDone),
    );

    p.push(Item::Heading("Output".into()), None);
    p.push(
        Item::Choice {
            label: "Also write".into(),
            chips: vec![
                Chip::gated("KFX", cfg.pack_kfx, converter),
                Chip::gated("EPUB", cfg.convert_epub, converter),
            ],
        },
        Some(Row::Convert),
    );
    p.push(Item::Note(output_note(converter)), None);

    p
}

/// One tap on chip `chip` of `row`.
///
/// [`Row::Formats`] is a pick: the chip tapped becomes the one that is on. The
/// rest are toggles.
fn apply(row: Row, chip: usize, cfg: &mut Config, folders: &[Candidate]) {
    match row {
        Row::Scan => {
            let Some(dir) = folders.get(chip).map(|c| c.dir.clone()) else {
                return;
            };
            match cfg.scan_dirs.iter().position(|d| *d == dir) {
                Some(at) => {
                    cfg.scan_dirs.remove(at);
                }
                // Appended, so the file keeps the order they were picked in
                // and `folders` keeps the order they are drawn in.
                None => cfg.scan_dirs.push(dir),
            }
        }
        Row::Formats => {
            if let Some((_, kfx, mobi)) = FORMAT_SETS.get(chip) {
                cfg.types_kfx = *kfx;
                cfg.types_mobi = *mobi;
            }
        }
        Row::Convert => match chip {
            0 => cfg.pack_kfx = !cfg.pack_kfx,
            _ => cfg.convert_epub = !cfg.convert_epub,
        },
        Row::ShowDone => cfg.show_done = !cfg.show_done,
    }
}

/// The line height of the title face, for [`Layout::compute`].
fn title_line_height(renderer: &mut TextRenderer) -> u32 {
    renderer.at_px(TITLE_PX, |r| r.line_height())
}

/// Owns input until a tap on the `[ Done ]` strip, mutating `cfg` in place.
///
/// `folders` is fixed for the life of the panel: it names what is on the
/// device, which a tap here does not change.
pub fn run(
    fb: &mut Framebuffer,
    input: &mut Input,
    renderer: &mut TextRenderer,
    cfg: &mut Config,
    folders: &[Candidate],
    orient: &mut Orientation,
    converter: bool,
) -> anyhow::Result<()> {
    let mut p = page(cfg, folders, converter);
    let mut layout = layout_for(renderer, fb.var.yres, &p.items);

    macro_rules! repaint {
        () => {{
            panel::render(
                fb,
                renderer,
                &layout,
                "Settings",
                status(converter),
                TITLE_PX,
                &p.items,
            );
            fb.send_update(panel::full_rect(fb), WAVEFORM_MODE_GC16)?;
        }};
    }

    repaint!();

    loop {
        match input.next_event()? {
            InputEvent::Touch(TouchEvent::Up { x, y }) => {
                if layout.done(y) {
                    return Ok(());
                }
                let Some(tap) = panel::hit(&p.items, &layout, fb.var.xres, x, y, |s| {
                    renderer.measure_width(s)
                }) else {
                    continue;
                };
                let Some(row) = p.row(tap.item) else {
                    continue;
                };

                let before = std::mem::take(&mut p.items);
                apply(row, tap.chip, cfg, folders);
                p = page(cfg, folders, converter);

                // The tapped row alone repaints when nothing else moved. The
                // Scan row rewrites the note under it, a hidden note changes
                // the row count, and grey does not survive the two-level DU
                // refresh a single row gets — any of those takes the panel.
                let alone = p.items.len() == before.len()
                    && !p.items[tap.item].has_quiet()
                    && (0..p.items.len()).all(|i| i == tap.item || p.items[i] == before[i]);
                if alone {
                    panel::draw_row(fb, renderer, &layout, &p.items, tap.item);
                    fb.send_update(layout.row_rect(tap.item, fb.var.xres), WAVEFORM_MODE_DU)?;
                } else {
                    layout = layout_for(renderer, fb.var.yres, &p.items);
                    repaint!();
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
                    layout = layout_for(renderer, fb.var.yres, &p.items);
                    repaint!();
                }
            }
            // `Input` holds the bezel keys grabbed; a press reaches no further.
            _ => {}
        }
    }
}

/// [`Layout::compute`] against the two faces this page draws.
fn layout_for(renderer: &mut TextRenderer, yres: u32, items: &[Item]) -> Layout {
    Layout::compute(
        renderer.line_height(),
        title_line_height(renderer),
        yres,
        items,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn dir(path: &str, books: usize) -> Candidate {
        Candidate {
            dir: PathBuf::from(path),
            books,
        }
    }

    /// The three folders a Kindle plausibly has, in `scan::candidates` order.
    fn folders() -> Vec<Candidate> {
        vec![
            dir(config::DOCUMENTS_DIR, 2),
            dir(config::ITEMS01_DIR, 12),
            dir("/mnt/us/documents/Sidle", 4),
        ]
    }

    /// Every chip of the page, as `(row, label, on, inert)`.
    fn chips(cfg: &Config, converter: bool) -> Vec<(Row, String, bool, bool)> {
        let p = page(cfg, &folders(), converter);
        let mut out = Vec::new();
        for (i, item) in p.items.iter().enumerate() {
            let (Item::Choice { chips, .. }, Some(row)) = (item, p.row(i)) else {
                continue;
            };
            for chip in chips {
                out.push((row, chip.label.clone(), chip.on, chip.inert));
            }
        }
        out
    }

    /// The labels of `row`'s filled chips.
    fn filled(cfg: &Config, row: Row) -> Vec<String> {
        chips(cfg, true)
            .into_iter()
            .filter(|(r, _, on, _)| *r == row && *on)
            .map(|(_, label, _, _)| label)
            .collect()
    }

    /// Tap chip `c` of `row`.
    fn tap(cfg: &mut Config, row: Row, c: usize) {
        apply(row, c, cfg, &folders());
    }

    #[test]
    fn every_item_either_changes_a_row_or_is_not_a_control() {
        for converter in [true, false] {
            let cfg = Config::default();
            let p = page(&cfg, &folders(), converter);
            assert_eq!(p.items.len(), p.rows.len());
            for (i, item) in p.items.iter().enumerate() {
                match item {
                    Item::Choice { .. } => assert!(p.row(i).is_some(), "item {i} changes nothing"),
                    Item::Heading(_) | Item::Note(_) => {
                        assert_eq!(p.row(i), None, "item {i} is not a control")
                    }
                }
            }
        }
    }

    #[test]
    fn every_row_of_the_page_is_reachable() {
        let cfg = Config::default();
        let p = page(&cfg, &folders(), true);
        for row in [Row::Scan, Row::Formats, Row::Convert, Row::ShowDone] {
            assert!(
                (0..p.items.len()).any(|i| p.row(i) == Some(row)),
                "{row:?} is not on the page"
            );
        }
    }

    #[test]
    fn the_scan_row_draws_one_chip_per_folder_on_the_device() {
        let cfg = Config::default();
        let labels: Vec<String> = chips(&cfg, true)
            .into_iter()
            .filter(|(row, ..)| *row == Row::Scan)
            .map(|(_, label, _, _)| label)
            .collect();
        // Named relative to the library root, which is what fits a row.
        assert_eq!(labels, ["documents", "Downloads/Items01", "Sidle"]);
        // A fresh install has the firmware's download folder picked.
        assert_eq!(filled(&cfg, Row::Scan), ["Downloads/Items01"]);
    }

    #[test]
    fn each_scan_chip_toggles_exactly_its_own_folder() {
        let before = Config::default();
        for (chip, folder) in folders().iter().enumerate() {
            let mut cfg = before.clone();
            tap(&mut cfg, Row::Scan, chip);
            assert_ne!(cfg, before, "{:?} changed nothing", folder.dir);
            assert_eq!(
                cfg.scan_dirs.contains(&folder.dir),
                !before.scan_dirs.contains(&folder.dir)
            );
            // Every other folder is where it was.
            for other in folders().iter().filter(|c| c.dir != folder.dir) {
                assert_eq!(
                    cfg.scan_dirs.contains(&other.dir),
                    before.scan_dirs.contains(&other.dir),
                    "{:?} moved with {:?}",
                    other.dir,
                    folder.dir
                );
            }
            // Tapping twice is the identity.
            tap(&mut cfg, Row::Scan, chip);
            assert_eq!(cfg, before);
        }
    }

    #[test]
    fn folders_can_be_scanned_together() {
        let mut cfg = Config::default();
        tap(&mut cfg, Row::Scan, 2); // Sidle, beside the download folder
        assert_eq!(filled(&cfg, Row::Scan), ["Downloads/Items01", "Sidle"]);
        assert_eq!(cfg.scan_dirs.len(), 2);
    }

    #[test]
    fn deselecting_every_folder_is_reachable_and_says_so() {
        let mut cfg = Config::default();
        tap(&mut cfg, Row::Scan, 1); // the one that was on
        assert!(cfg.scan_dirs.is_empty());
        assert!(filled(&cfg, Row::Scan).is_empty());
        assert!(!cfg.lists_anything());
        assert!(scan_note(&cfg, &folders()).contains("Nothing selected"));
    }

    #[test]
    fn the_scan_note_counts_the_books_the_grid_would_show() {
        let mut cfg = Config::default();
        // The default picks `Downloads/Items01`, which holds 12 of the 18.
        assert_eq!(
            scan_note(&cfg, &folders()),
            "12 DRM'd books in 1 of 3 folders"
        );
        tap(&mut cfg, Row::Scan, 2);
        assert_eq!(
            scan_note(&cfg, &folders()),
            "16 DRM'd books in 2 of 3 folders"
        );
        // One folder on the device: the count alone, with nothing to pick from.
        let only = vec![dir(config::ITEMS01_DIR, 31)];
        assert_eq!(scan_note(&cfg, &only), "31 DRM'd books");
        let one = vec![dir(config::ITEMS01_DIR, 1)];
        assert_eq!(scan_note(&cfg, &one), "1 DRM'd book");
    }

    #[test]
    fn the_format_chips_are_a_pick_and_leave_exactly_one_filled() {
        let mut cfg = Config::default();
        for (chip, (label, ..)) in FORMAT_SETS.iter().enumerate() {
            tap(&mut cfg, Row::Formats, chip);
            assert_eq!(filled(&cfg, Row::Formats), [*label], "chip {chip}");
            // KFX is on under either, so no pick can empty the grid by format.
            assert!(cfg.types_kfx, "chip {chip} turned every format off");
            assert!(cfg.lists_anything());
        }
    }

    #[test]
    fn a_hand_edited_format_pair_the_page_has_no_chip_for_fills_none() {
        // `types_kfx = false` is reachable through the file and through no
        // chip; the row says so by filling nothing rather than by lying.
        let cfg = Config {
            types_kfx: false,
            types_mobi: true,
            ..Config::default()
        };
        assert!(filled(&cfg, Row::Formats).is_empty());
    }

    #[test]
    fn each_convert_chip_toggles_exactly_its_own_format() {
        let before = Config::default();
        for chip in 0..2 {
            let mut cfg = before.clone();
            tap(&mut cfg, Row::Convert, chip);
            if chip == 0 {
                assert_ne!(cfg.pack_kfx, before.pack_kfx);
                assert_eq!(cfg.convert_epub, before.convert_epub);
            } else {
                assert_eq!(cfg.pack_kfx, before.pack_kfx);
                assert_ne!(cfg.convert_epub, before.convert_epub);
            }
            tap(&mut cfg, Row::Convert, chip);
            assert_eq!(cfg, before);
        }
    }

    #[test]
    fn the_convert_chips_are_grey_and_untappable_without_the_add_on() {
        let cfg = Config {
            pack_kfx: true,
            convert_epub: false,
            ..Config::default()
        };
        let convert: Vec<(String, bool, bool)> = chips(&cfg, false)
            .into_iter()
            .filter(|(row, ..)| *row == Row::Convert)
            .map(|(_, label, on, inert)| (label, on, inert))
            .collect();
        assert_eq!(
            convert,
            [
                ("KFX".to_string(), true, true),
                ("EPUB".to_string(), false, true)
            ]
        );
        // Installed, the same settings are live and read the same way.
        assert!(
            chips(&cfg, true)
                .iter()
                .filter(|(row, ..)| *row == Row::Convert)
                .all(|(_, _, _, inert)| !inert)
        );
        assert_eq!(filled(&cfg, Row::Convert), ["KFX"]);
    }

    #[test]
    fn the_output_note_names_the_folder_or_the_add_on_that_is_missing() {
        let installed = output_note(true);
        assert!(installed.contains(config::OUT_DIR));
        assert_eq!(installed.lines().count(), 1);

        // The install lines take the row instead; `ui::header` is where the
        // destination is named on every other screen.
        let missing = output_note(false);
        assert!(missing.contains(convert::RELEASES_URL));
        assert!(missing.contains(convert::RELEASE_ASSET));
        assert!(missing.contains(convert::EXTENSION_DIR));
        assert_eq!(missing.lines().count(), 2);
    }

    #[test]
    fn a_folder_outside_the_library_root_keeps_its_whole_path() {
        // Reachable by hand-editing the file; `scan::candidates` keeps it on
        // the page so it can be switched off again.
        let elsewhere = PathBuf::from("/mnt/base-us/books");
        let cfg = Config {
            scan_dirs: vec![elsewhere.clone()],
            ..Config::default()
        };
        let folders = vec![dir("/mnt/base-us/books", 3)];
        let p = page(&cfg, &folders, true);
        let Item::Choice { chips, .. } = &p.items[1] else {
            unreachable!()
        };
        assert_eq!(chips[0].label, "/mnt/base-us/books");
        assert!(chips[0].on);
    }

    #[test]
    fn no_note_line_runs_off_the_narrowest_panel() {
        let mut cfg = Config::default();
        let mut lines: Vec<String> = output_note(false).lines().map(str::to_string).collect();
        // Every count the scan note can carry, including none selected.
        for picks in [0usize, 1, 3] {
            cfg.scan_dirs = folders()
                .iter()
                .take(picks)
                .map(|c| c.dir.clone())
                .collect();
            lines.push(scan_note(&cfg, &folders()));
        }
        // A folder deep enough to be worth checking against the same budget.
        lines.push(
            dir("/mnt/us/documents/Downloads/Items01", 1)
                .label()
                .to_string(),
        );
        for line in lines {
            let n = line.chars().count();
            assert!(
                n <= NOTE_MAX_CHARS,
                "{n} chars, over {NOTE_MAX_CHARS}: {line}"
            );
        }
    }

    #[test]
    fn the_status_line_asks_for_the_add_on_only_when_it_is_missing() {
        assert_ne!(status(true), status(false));
        assert!(!status(true).contains("add-on"));
        assert!(status(false).contains("add-on"));
    }

    /// The page has to stay a page: `ui::panel` cuts rows at the strip.
    #[test]
    fn the_whole_page_fits_the_shortest_panel() {
        let yres = crate::ui::pager::NARROWEST_PANEL_W;
        for converter in [true, false] {
            let p = page(&Config::default(), &folders(), converter);
            for lh in 24..=64 {
                let layout = Layout::compute(lh, lh * 4 / 3, yres, &p.items);
                assert_eq!(
                    layout.drawable(),
                    p.items.len(),
                    "lh={lh} converter={converter}"
                );
            }
        }
    }

    #[test]
    fn a_candidate_is_named_relative_to_the_library_root() {
        assert_eq!(dir(config::DOCUMENTS_DIR, 1).label(), "documents");
        assert_eq!(dir(config::ITEMS01_DIR, 1).label(), "Downloads/Items01");
        assert_eq!(dir("/mnt/us/documents/Sidle", 1).label(), "Sidle");
        assert_eq!(dir("/elsewhere/books", 1).label(), "/elsewhere/books");
        assert!(Path::new(config::ITEMS01_DIR).starts_with(config::DOCUMENTS_DIR));
    }
}
