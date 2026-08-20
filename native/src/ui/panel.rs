//! A settings page: section headings, and rows whose values sit side by side
//! as chips.
//!
//! **A settings page is not a list.** Every value a setting can take is on the
//! panel at once, in its own tap target, so reading the page and changing it
//! are the same gesture.
//!
//! - **Geometry comes from the font.** Row heights are multiples of the line
//!   height with a tap-target floor, so a larger face gives larger targets
//!   instead of a broken layout. [`Layout::compute`] takes the first [`TIERS`]
//!   entry whose rows finish above the strip.
//! - **One column for every row's values**, measured from the widest label on
//!   the page, so the chips line up down the page and it reads as a table.
//! - **Hit-testing over a measured layout**, free of the framebuffer and the
//!   font: [`chip_bounds`] is the single source for drawing and for [`hit`],
//!   so a finger can never land on a chip other than the one under it.
//!
//! [`crate::ui::configmenu`] builds the [`Item`]s and owns what a tap means.

use crate::eink::fb::{Framebuffer, MxcfbRect};
use crate::ui::text::TextRenderer;
use crate::ui::{BLACK, QUIET, WHITE};

/// Bottom action strip, matching `setup::BTN_H`.
pub const STRIP_H: u32 = 120;
/// Left inset for the title, the headings and the strip's label.
pub const MARGIN_X: u32 = 60;
/// Where a row's own label starts, inside the margin.
pub const ROW_INSET: u32 = MARGIN_X + 24;

/// Blank space either side of a chip's text.
const CHIP_PAD: u32 = 24;
/// Between one chip and the next.
const CHIP_GAP: u32 = 20;
/// Chip height as a percentage of its row's.
const CHIP_H_PCT: u32 = 72;
/// Border of an unfilled chip.
const CHIP_BORDER: u32 = 2;
/// The rule under a [`Item::Heading`].
const RULE_H: u32 = 3;

/// One value a row can take, and its own tap target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub label: String,
    /// What the setting is currently on. Drawn filled.
    pub on: bool,
    /// Says where the row stands rather than offering to change it: drawn in
    /// [`QUIET`] and refused by [`hit`].
    ///
    /// The word stays so the row still reads as a row; the grey is what says
    /// it is not a control.
    pub inert: bool,
}

impl Chip {
    pub fn new(label: impl Into<String>, on: bool) -> Self {
        Chip {
            label: label.into(),
            on,
            inert: false,
        }
    }

    /// [`Chip::new`], greyed out and untappable when `live` is false.
    pub fn gated(label: impl Into<String>, on: bool, live: bool) -> Self {
        Chip {
            inert: !live,
            ..Chip::new(label, on)
        }
    }
}

/// One line of the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A section heading with a rule under it. Names what follows; not
    /// tappable.
    Heading(String),
    /// A setting: a label, and the values it can take drawn beside it.
    Choice { label: String, chips: Vec<Chip> },
    /// Quiet text under the row it explains, one row per `\n`-delimited line.
    /// Not tappable, and drawn from [`ROW_INSET`] rather than the chip column
    /// so a long line has the width of the page.
    Note(String),
}

impl Item {
    /// The label the chip column is measured against. `None` for a line that
    /// has no second column and so constrains nothing.
    fn label(&self) -> Option<&str> {
        match self {
            Item::Choice { label, .. } => Some(label),
            Item::Heading(_) | Item::Note(_) => None,
        }
    }

    /// Whether this item draws anything in [`QUIET`].
    ///
    /// A `WAVEFORM_MODE_DU` region is two-level and snaps mid-grey to black or
    /// white, so a caller refreshing one row has to take the whole panel with
    /// a `WAVEFORM_MODE_GC16` when this is true.
    pub fn has_quiet(&self) -> bool {
        match self {
            Item::Note(_) => true,
            Item::Choice { chips, .. } => chips.iter().any(|c| c.inert),
            Item::Heading(_) => false,
        }
    }

    /// Lines of text stacked inside this item's row.
    fn lines(&self) -> u32 {
        match self {
            Item::Note(text) => text.lines().count().max(1) as u32,
            _ => 1,
        }
    }
}

/// How generous the row heights are, as a percentage of the line height.
///
/// [`Layout::compute`] takes the first entry whose rows finish above the
/// strip. A panel held in landscape is short, and a row past the strip is
/// neither drawn nor tappable, so the page gives up air before it gives up
/// rows.
struct Tier {
    heading: u32,
    choice: u32,
    /// Per line of a [`Item::Note`].
    note: u32,
    /// Least height a [`Item::Choice`] row may take, whatever the face
    /// measures: a chip is [`CHIP_H_PCT`] of it and has to stay a finger
    /// target on a ~300 DPI panel.
    floor: u32,
    /// Air above the first row, as a percentage of the line height.
    air: u32,
}

const TIERS: [Tier; 3] = [
    Tier {
        heading: 200,
        choice: 200,
        note: 150,
        floor: 96,
        air: 200,
    },
    Tier {
        heading: 160,
        choice: 170,
        note: 125,
        floor: 88,
        air: 150,
    },
    Tier {
        heading: 120,
        choice: 130,
        note: 100,
        floor: 72,
        air: 100,
    },
];

/// One row's slice of the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RowBox {
    top: u32,
    height: u32,
}

/// Vertical geometry, derived from the faces actually drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Body line height.
    lh: u32,
    title_top: u32,
    status_top: u32,
    rows_top: u32,
    rows: Vec<RowBox>,
    strip_top: u32,
}

impl Layout {
    /// `lh` and `title_lh` are the line heights of the two faces actually
    /// drawn; spacing derived from the wrong one overlaps the title and the
    /// status line.
    pub fn compute(lh: u32, title_lh: u32, yres: u32, items: &[Item]) -> Self {
        let lh = lh.max(1);
        let title_lh = title_lh.max(1);
        let title_top = title_lh / 2;
        let status_top = title_top + title_lh;
        let strip_top = yres.saturating_sub(STRIP_H);

        // The last tier stands whether or not it fits: a page with nowhere to
        // put its rows is still better drawn tight than not drawn.
        let last = TIERS.len() - 1;
        let (rows_top, rows) = (0..TIERS.len())
            .map(|t| plan(&TIERS[t], lh, status_top + lh, items))
            .enumerate()
            .find(|(t, (_, rows))| *t == last || fits(rows, strip_top))
            .map(|(_, planned)| planned)
            .expect("TIERS is not empty");

        Layout {
            lh,
            title_top,
            status_top,
            rows_top,
            rows,
            strip_top,
        }
    }

    /// Which item a tap at `y` fell on, or `None` above the rows, in the gap
    /// past the last one, or on the strip.
    pub fn row_at(&self, y: u32) -> Option<usize> {
        if y < self.rows_top || y >= self.strip_top {
            return None;
        }
        self.rows
            .iter()
            .position(|b| y >= b.top && y < b.top + b.height)
    }

    /// True on the `[ Done ]` strip.
    pub fn done(&self, y: u32) -> bool {
        y >= self.strip_top
    }

    /// The rows that finish above the strip. A page longer than the panel is
    /// cut here rather than drawn over its own strip.
    pub fn drawable(&self) -> usize {
        self.rows
            .iter()
            .take_while(|b| b.top + b.height <= self.strip_top)
            .count()
    }

    /// One row's rect, for a single-row refresh.
    pub fn row_rect(&self, item: usize, xres: u32) -> MxcfbRect {
        let b = self.rows[item];
        MxcfbRect {
            top: b.top,
            left: 0,
            width: xres,
            height: b.height,
        }
    }
}

/// [`Layout::rows`] for one [`Tier`], and the `rows_top` it starts at.
fn plan(tier: &Tier, lh: u32, below_status: u32, items: &[Item]) -> (u32, Vec<RowBox>) {
    let rows_top = below_status + lh * tier.air / 100;
    let mut y = rows_top;
    let rows = items
        .iter()
        .map(|item| {
            let height = match item {
                Item::Heading(_) => lh * tier.heading / 100,
                Item::Choice { .. } => (lh * tier.choice / 100).max(tier.floor),
                Item::Note(_) => lh * tier.note / 100 * item.lines(),
            };
            let top = y;
            y += height;
            RowBox { top, height }
        })
        .collect();
    (rows_top, rows)
}

fn fits(rows: &[RowBox], strip_top: u32) -> bool {
    rows.last().is_none_or(|b| b.top + b.height <= strip_top)
}

/// How wide each chip of a row is: its own text, and the blank either side.
fn chip_widths(chips: &[Chip], measure: &mut impl FnMut(&str) -> u32) -> Vec<u32> {
    chips
        .iter()
        .map(|c| measure(&c.label).saturating_add(CHIP_PAD * 2))
        .collect()
}

/// How wide a run of chips is once tiled, the gaps between them included.
///
/// The one place that arithmetic lives, because [`chip_column`] has to know
/// what [`chip_bounds`] is about to lay out: a second copy of it would leave
/// room for a run of a different length than the one drawn.
fn run_width(widths: impl IntoIterator<Item = u32>) -> u32 {
    let mut total = 0u32;
    let mut cells = 0u32;
    for w in widths {
        total = total.saturating_add(w);
        cells = cells.saturating_add(1);
    }
    total.saturating_add(CHIP_GAP.saturating_mul(cells.saturating_sub(1)))
}

/// Where every row's chips start, in screen x.
///
/// One column across the whole page, from the widest label on it, so the
/// values line up instead of stepping in and out behind labels of different
/// lengths — which is most of what makes a settings page read as a table.
///
/// Measured against the panel it is drawn on: the column is pulled back far
/// enough that the widest run on the page still finishes inside the right
/// margin, and floored at a third of the line so one long run cannot squeeze
/// every label on the page.
pub fn chip_column(items: &[Item], xres: u32, mut measure: impl FnMut(&str) -> u32) -> u32 {
    let widest = items
        .iter()
        .filter_map(|item| item.label().map(&mut measure))
        .max()
        .unwrap_or(0);
    let wanted = ROW_INSET
        .saturating_add(widest)
        .saturating_add(CHIP_GAP * 3);

    let right = xres.saturating_sub(MARGIN_X);
    let room = items
        .iter()
        .filter_map(|item| match item {
            Item::Choice { chips, .. } => {
                Some(right.saturating_sub(run_width(chip_widths(chips, &mut measure))))
            }
            // Neither has a second column, so neither constrains one.
            Item::Heading(_) | Item::Note(_) => None,
        })
        .min()
        .unwrap_or(u32::MAX);

    wanted.min(room.max(xres / 3).min(xres / 2))
}

/// Where each chip of one row sits, as `(x, width)` pairs.
///
/// **The single source for drawing and for [`hit`]**: a chip is only as wide
/// as its own text, so anything that measured them a second time would put a
/// finger on a different one. A chip that would run past the right margin is
/// dropped rather than shrunk.
///
/// **The shared column is a courtesy, and a row that cannot afford it keeps
/// its own.** [`chip_column`] is floored at a third of the line to keep labels
/// readable, so a row with a long run and a short label can still overflow it;
/// such a row starts its chips just past its own label instead. That steps it
/// out of the table, which is the trade: a control that is off the page is
/// worse than a row out of line with its neighbours.
pub fn chip_bounds(
    column: u32,
    xres: u32,
    label: &str,
    chips: &[Chip],
    mut measure: impl FnMut(&str) -> u32,
) -> Vec<(u32, u32)> {
    let right = xres.saturating_sub(MARGIN_X);
    let widths = chip_widths(chips, &mut measure);
    let wanted = run_width(widths.iter().copied());
    // Its own label only when that is further left than the column: the row
    // the column was measured from gains nothing by starting after it again.
    let mut x = if column.saturating_add(wanted) <= right {
        column
    } else {
        (ROW_INSET + measure(label) + CHIP_GAP).min(column)
    };

    let mut out = Vec::new();
    for w in widths {
        if x.saturating_add(w) > right {
            break;
        }
        out.push((x, w));
        x = x.saturating_add(w + CHIP_GAP);
    }
    out
}

/// The rect of one chip, given the bounds its row was laid out at.
fn chip_rect(layout: &Layout, item: usize, bounds: &[(u32, u32)], option: usize) -> MxcfbRect {
    let (left, width) = bounds.get(option).copied().unwrap_or((0, 0));
    let b = layout.rows[item];
    let height = b.height * CHIP_H_PCT / 100;
    MxcfbRect {
        top: b.top + (b.height - height) / 2,
        left,
        width,
        height,
    }
}

/// What a tap landed on: which item, and which of its chips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub item: usize,
    pub chip: usize,
}

/// The chip at `(x, y)`.
///
/// `None` for a heading, a note, a row's label, an inert chip, and the space
/// between and past the chips. None of them is a control, and a settings page
/// where the gaps do something is one you cannot rest a hand on.
pub fn hit(
    items: &[Item],
    layout: &Layout,
    xres: u32,
    x: u32,
    y: u32,
    mut measure: impl FnMut(&str) -> u32,
) -> Option<Hit> {
    let item = layout.row_at(y)?;
    let Item::Choice { label, chips } = items.get(item)? else {
        return None;
    };
    let column = chip_column(items, xres, &mut measure);
    let bounds = chip_bounds(column, xres, label, chips, &mut measure);
    let chip = bounds
        .iter()
        .position(|(left, width)| x >= *left && x < left + width)?;
    (!chips[chip].inert).then_some(Hit { item, chip })
}

/// Point-in-rect, in screen coords.
fn inside(rect: MxcfbRect, x: u32, y: u32) -> bool {
    x >= rect.left && x < rect.left + rect.width && y >= rect.top && y < rect.top + rect.height
}

/// The whole page: title, status line, rows, strip.
pub fn render(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    layout: &Layout,
    title: &str,
    status: &str,
    title_px: f32,
    items: &[Item],
) {
    let xres = fb.var.xres;
    fb.fill_rect(0, 0, xres, fb.var.yres, WHITE);

    renderer.at_px(title_px, |r| {
        let baseline = (layout.title_top + r.line_height() * 78 / 100) as i32;
        r.draw_bold(fb, MARGIN_X as i32, baseline, title, BLACK);
    });
    if !status.is_empty() {
        let baseline = (layout.status_top + layout.lh * 78 / 100) as i32;
        renderer.draw_ink(fb, MARGIN_X as i32, baseline, status, QUIET);
    }

    let column = chip_column(items, xres, |s| renderer.measure_width(s));
    for i in 0..layout.drawable() {
        draw_item(fb, renderer, layout, items, i, column);
    }
    draw_strip(fb, renderer, layout);
}

/// Redraw one row in place, for a chip that changed. The rest of the page is
/// left alone: a full-panel refresh to invert one chip is half a second of ink.
pub fn draw_row(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    layout: &Layout,
    items: &[Item],
    item: usize,
) {
    let column = chip_column(items, fb.var.xres, |s| renderer.measure_width(s));
    draw_item(fb, renderer, layout, items, item, column);
}

fn draw_item(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    layout: &Layout,
    items: &[Item],
    i: usize,
    column: u32,
) {
    let xres = fb.var.xres;
    let Some(item) = items.get(i) else {
        return;
    };
    let b = layout.rows[i];
    fb.fill_rect(b.top, 0, xres, b.height, WHITE);
    let middle = (b.top + b.height / 2) as i32;
    let baseline = |lh: u32| middle + (lh as i32 * 36 / 100);

    match item {
        // The text sits at the foot of its row with the rule directly under
        // it, so the empty half above reads as the gap between sections.
        Item::Heading(text) => {
            let foot = b.top + b.height.saturating_sub(RULE_H);
            let text_baseline = foot.saturating_sub(layout.lh / 5) as i32;
            renderer.draw_bold(fb, MARGIN_X as i32, text_baseline, text, BLACK);
            fb.fill_rect(
                foot,
                MARGIN_X,
                xres.saturating_sub(MARGIN_X * 2),
                RULE_H,
                BLACK,
            );
        }
        // No rule of its own: the chips are visibly bounded already, and a
        // line under every setting buries the structure of the page.
        Item::Choice { label, chips } => {
            renderer.draw(fb, ROW_INSET as i32, baseline(layout.lh), label, false);
            let bounds = chip_bounds(column, xres, label, chips, |s| renderer.measure_width(s));
            for (c, chip) in chips.iter().enumerate().take(bounds.len()) {
                draw_chip(fb, renderer, chip_rect(layout, i, &bounds, c), chip);
            }
        }
        Item::Note(text) => {
            let lines = item.lines();
            let per = b.height / lines.max(1);
            for (n, line) in text.lines().enumerate() {
                let top = b.top + n as u32 * per;
                let y = (top + per / 2) as i32 + (layout.lh as i32 * 36 / 100);
                renderer.draw_ink(fb, ROW_INSET as i32, y, line, QUIET);
            }
        }
    }
}

/// One chip: filled when it is what the setting is on, outlined when it is
/// merely available, grey when it is neither.
///
/// **Filled, not ticked.** `ui::text` cuts glyph coverage to one bit, so a
/// tick is a smudge at this size; an inverted block is unambiguous, and it is
/// the idiom the rest of the app already marks state with.
fn draw_chip(fb: &mut Framebuffer, renderer: &mut TextRenderer, rect: MxcfbRect, chip: &Chip) {
    let (ground, ink) = match (chip.on, chip.inert) {
        (true, _) => (BLACK, WHITE),
        // Border and word both recede: a chip that cannot be pressed drawn in
        // the same black as one that can is the page offering an action it
        // does not have.
        (false, true) => (WHITE, QUIET),
        (false, false) => (WHITE, BLACK),
    };
    fb.fill_rect(rect.top, rect.left, rect.width, rect.height, ground);
    if !chip.on {
        let t = CHIP_BORDER;
        fb.fill_rect(rect.top, rect.left, rect.width, t, ink);
        fb.fill_rect(rect.top + rect.height - t, rect.left, rect.width, t, ink);
        fb.fill_rect(rect.top, rect.left, t, rect.height, ink);
        fb.fill_rect(rect.top, rect.left + rect.width - t, t, rect.height, ink);
    }

    let w = renderer.measure_width(&chip.label);
    let x = rect.left as i32 + ((rect.width as i32 - w as i32) / 2).max(0);
    let lh = renderer.line_height();
    let y = (rect.top + rect.height / 2) as i32 + (lh as i32 * 36 / 100);
    renderer.draw_ink(fb, x, y, &chip.label, ink);
}

/// The full-width `[ Done ]` row under a 2px rule.
fn draw_strip(fb: &mut Framebuffer, renderer: &mut TextRenderer, layout: &Layout) {
    let xres = fb.var.xres;
    let top = layout.strip_top;
    fb.fill_rect(top, 0, xres, 2, BLACK);
    fb.fill_rect(top + 2, 0, xres, STRIP_H - 2, WHITE);

    let label = "[ Done ]";
    let baseline = (top + STRIP_H * 60 / 100) as i32;
    renderer.draw(fb, MARGIN_X as i32, baseline, label, false);
}

/// The whole panel, for a `WAVEFORM_MODE_GC16` refresh.
pub fn full_rect(fb: &Framebuffer) -> MxcfbRect {
    MxcfbRect {
        top: 0,
        left: 0,
        width: fb.var.xres,
        height: fb.var.yres,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed-width face: every character is 20px. Lets the geometry be
    /// reasoned about arithmetically, with no font on the machine.
    fn measure(s: &str) -> u32 {
        s.chars().count() as u32 * 20
    }

    fn choice(label: &str, chips: &[&str]) -> Item {
        Item::Choice {
            label: label.to_string(),
            chips: chips.iter().map(|c| Chip::new(*c, false)).collect(),
        }
    }

    fn page() -> Vec<Item> {
        vec![
            Item::Heading("Books".into()),
            choice("Scan", &["Purchases", "Library"]),
            Item::Note("under /mnt/us".into()),
            choice("Also write", &["KFX", "EPUB"]),
        ]
    }

    const XRES: u32 = 1264;
    const YRES: u32 = 1680;

    fn layout(items: &[Item]) -> Layout {
        Layout::compute(44, 58, YRES, items)
    }

    #[test]
    fn every_row_starts_where_the_one_above_it_ends() {
        let items = page();
        let l = layout(&items);
        assert_eq!(l.rows.len(), items.len());
        assert_eq!(l.rows[0].top, l.rows_top);
        assert!(
            l.rows
                .windows(2)
                .all(|w| w[0].top + w[0].height == w[1].top),
            "gap between rows: {:?}",
            l.rows
        );
        // Contiguous, so `row_at` finds a row for every y over the block.
        let last = l.rows[items.len() - 1];
        for y in l.rows_top..last.top + last.height {
            assert!(l.row_at(y).is_some(), "no row at {y}");
        }
    }

    #[test]
    fn a_note_is_as_tall_as_the_lines_it_holds() {
        let one = vec![Item::Note("a".into())];
        let two = vec![Item::Note("a\nb".into())];
        assert_eq!(layout(&two).rows[0].height, layout(&one).rows[0].height * 2);
    }

    #[test]
    fn nothing_above_the_rows_or_on_the_strip_is_a_row() {
        let items = page();
        let l = layout(&items);
        assert_eq!(l.row_at(l.rows_top - 1), None);
        assert_eq!(l.row_at(l.strip_top), None);
        assert!(l.done(l.strip_top));
        assert!(!l.done(l.strip_top - 1));
        // The air between the last row and the strip belongs to neither.
        let last = l.rows[items.len() - 1];
        assert_eq!(l.row_at(last.top + last.height), None);
    }

    /// A panel held in landscape is the short one, and the page has to fit on
    /// it rather than run under its own strip.
    #[test]
    fn the_tiers_fit_the_page_onto_the_shortest_panel() {
        let items = page();
        // Brackets what `app::FONT_PX` produces across the faces a Kindle
        // ships, at both ends.
        for lh in 24..=64 {
            let yres = crate::ui::pager::NARROWEST_PANEL_W;
            let l = Layout::compute(lh, lh * 4 / 3, yres, &items);
            let last = l.rows[items.len() - 1];
            assert!(
                last.top + last.height <= l.strip_top,
                "lh={lh}: rows end at {} against a strip at {}",
                last.top + last.height,
                l.strip_top
            );
            assert_eq!(l.drawable(), items.len());
        }
    }

    #[test]
    fn a_page_longer_than_the_panel_is_cut_at_the_strip() {
        let items: Vec<Item> = (0..40).map(|_| choice("Scan", &["On"])).collect();
        let l = layout(&items);
        assert!(l.drawable() < items.len());
        // Every drawable row is above the strip, and the first one that is not
        // is where drawing stops.
        for i in 0..l.drawable() {
            assert!(l.rows[i].top + l.rows[i].height <= l.strip_top);
        }
        assert!(l.rows[l.drawable()].top + l.rows[l.drawable()].height > l.strip_top);
    }

    #[test]
    fn the_chips_of_every_row_start_at_one_column() {
        let items = page();
        let column = chip_column(&items, XRES, measure);
        // The widest label on the page sets it: `Also write` at 10 chars.
        assert_eq!(column, ROW_INSET + measure("Also write") + CHIP_GAP * 3);
        for item in &items {
            let Item::Choice { label, chips } = item else {
                continue;
            };
            let bounds = chip_bounds(column, XRES, label, chips, measure);
            assert_eq!(bounds[0].0, column, "{label} is out of the column");
        }
    }

    #[test]
    fn a_chip_is_its_own_text_plus_the_blank_either_side() {
        let items = page();
        let column = chip_column(&items, XRES, measure);
        let bounds = chip_bounds(
            column,
            XRES,
            "Also write",
            &[Chip::new("KFX", false)],
            measure,
        );
        assert_eq!(bounds[0].1, measure("KFX") + CHIP_PAD * 2);
    }

    #[test]
    fn a_run_too_wide_for_the_column_starts_after_its_own_label() {
        // A short label and a run that cannot fit beside the shared column.
        let long = "/mnt/us/documents/dedrm";
        let items = vec![
            choice("Some very long label indeed", &["a"]),
            choice("Out", &[long, long]),
        ];
        let column = chip_column(&items, XRES, measure);
        let Item::Choice { label, chips } = &items[1] else {
            unreachable!()
        };
        let bounds = chip_bounds(column, XRES, label, chips, measure);
        assert!(!bounds.is_empty(), "the row lost every chip");
        assert!(
            bounds[0].0 < column,
            "started at the column it could not afford"
        );
        assert_eq!(bounds[0].0, ROW_INSET + measure(label) + CHIP_GAP);
    }

    #[test]
    fn a_chip_past_the_right_margin_is_dropped_rather_than_shrunk() {
        let chips: Vec<Chip> = (0..12)
            .map(|i| Chip::new(format!("chip{i}"), false))
            .collect();
        let items = vec![Item::Choice {
            label: "Row".into(),
            chips: chips.clone(),
        }];
        let column = chip_column(&items, XRES, measure);
        let bounds = chip_bounds(column, XRES, "Row", &chips, measure);
        assert!(bounds.len() < chips.len());
        // Whatever survived finishes inside the margin at full width.
        for (x, w) in &bounds {
            assert!(x + w <= XRES - MARGIN_X);
            assert_eq!(*w, measure("chip0") + CHIP_PAD * 2);
        }
    }

    #[test]
    fn a_tap_lands_on_the_chip_it_is_over_and_nowhere_else() {
        let items = page();
        let l = layout(&items);
        let column = chip_column(&items, XRES, measure);
        let Item::Choice { label, chips } = &items[1] else {
            unreachable!()
        };
        let bounds = chip_bounds(column, XRES, label, chips, measure);

        for (c, (x, w)) in bounds.iter().enumerate() {
            let y = l.rows[1].top + l.rows[1].height / 2;
            assert_eq!(
                hit(&items, &l, XRES, x + w / 2, y, measure),
                Some(Hit { item: 1, chip: c })
            );
        }
        // The label, the gap between two chips, and past the last one.
        let y = l.rows[1].top + l.rows[1].height / 2;
        assert_eq!(hit(&items, &l, XRES, ROW_INSET + 4, y, measure), None);
        let gap = bounds[0].0 + bounds[0].1 + CHIP_GAP / 2;
        assert_eq!(hit(&items, &l, XRES, gap, y, measure), None);
        let past = bounds[1].0 + bounds[1].1 + 40;
        assert_eq!(hit(&items, &l, XRES, past, y, measure), None);
    }

    #[test]
    fn a_heading_and_a_note_take_no_taps() {
        let items = page();
        let l = layout(&items);
        let column = chip_column(&items, XRES, measure);
        for row in [0usize, 2] {
            let y = l.rows[row].top + l.rows[row].height / 2;
            // Over the column, where a `Choice` row would have a chip.
            assert_eq!(hit(&items, &l, XRES, column + 10, y, measure), None);
        }
    }

    #[test]
    fn an_inert_chip_is_laid_out_and_not_tappable() {
        let items = vec![Item::Choice {
            label: "Also write".into(),
            chips: vec![
                Chip::gated("KFX", false, false),
                Chip::gated("EPUB", true, false),
            ],
        }];
        let l = layout(&items);
        let column = chip_column(&items, XRES, measure);
        let Item::Choice { chips, .. } = &items[0] else {
            unreachable!()
        };
        let bounds = chip_bounds(column, XRES, "Also write", chips, measure);
        assert_eq!(bounds.len(), 2, "an inert chip still takes its place");
        for (x, w) in &bounds {
            let y = l.rows[0].top + l.rows[0].height / 2;
            assert_eq!(hit(&items, &l, XRES, x + w / 2, y, measure), None);
        }
    }

    #[test]
    fn only_the_items_drawn_in_grey_say_so() {
        assert!(!Item::Heading("Books".into()).has_quiet());
        assert!(Item::Note("under /mnt/us".into()).has_quiet());
        assert!(!choice("Scan", &["Purchases"]).has_quiet());
        assert!(
            Item::Choice {
                label: "Also write".into(),
                chips: vec![Chip::new("KFX", true), Chip::gated("EPUB", false, false)],
            }
            .has_quiet()
        );
    }

    #[test]
    fn a_gated_chip_is_live_only_when_its_gate_is() {
        assert!(!Chip::gated("KFX", true, true).inert);
        assert!(Chip::gated("KFX", true, false).inert);
        // The gate does not decide what the setting is on.
        assert!(Chip::gated("KFX", true, false).on);
    }

    #[test]
    fn one_row_refreshes_across_the_panel_and_only_its_own_height() {
        let items = page();
        let l = layout(&items);
        let rect = l.row_rect(1, XRES);
        assert_eq!(rect.left, 0);
        assert_eq!(rect.width, XRES);
        assert_eq!(rect.top, l.rows[1].top);
        assert_eq!(rect.height, l.rows[1].height);
        // A chip's rect is inside it, so the refresh covers whatever changed.
        let column = chip_column(&items, XRES, measure);
        let Item::Choice { label, chips } = &items[1] else {
            unreachable!()
        };
        let bounds = chip_bounds(column, XRES, label, chips, measure);
        let chip = chip_rect(&l, 1, &bounds, 0);
        assert!(inside(rect, chip.left, chip.top));
        assert!(inside(
            rect,
            chip.left + chip.width - 1,
            chip.top + chip.height - 1
        ));
    }
}
