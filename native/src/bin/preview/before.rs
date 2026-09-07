//! The grid screen through the geometry of `551528c`, for the `--before` sheet:
//! every constant a device pixel with no density behind it, and the toolbar's
//! zones at fixed offsets. Drawing the same fixture both ways is what makes a
//! shot from `ui` comparable against one from a device on that build.
//!
//! **Frozen at that commit.** It is a copy of code that has since changed, and
//! keeping it in step with `ui` would leave nothing to compare against.

use kfxdedrm_fe_native::eink::fb::Framebuffer;
use kfxdedrm_fe_native::scan::Book;
use kfxdedrm_fe_native::ui::text::TextRenderer;

/// `ui::header`'s band, in device pixels.
const HEADER_TOP: u32 = 16;
const HEADER_H: u32 = 88;
const HEADER_MARGIN_X: u32 = 40;
const HEADER_MARGIN: u32 = HEADER_TOP + HEADER_H + 16;

/// `ui::grid`'s cell, in device pixels.
const CELL_W: u32 = 360;
const CELL_H_MIN: u32 = 420;
const CELL_H_MAX: u32 = 440;
const COL_GAP: u32 = 32;
const ROW_GAP: u32 = 20;
const NAME_BAND_H: u32 = 64;

/// `ui::pager`'s strip, in device pixels, with the zones at fixed offsets.
const STRIP_H: u32 = 80;
const EXIT_ZONE_W: u32 = 180;
const SETTINGS_ZONE_W: u32 = 240;
const ALL_LEFT: u32 = EXIT_ZONE_W + SETTINGS_ZONE_W;
const NAV_LEFT: u32 = ALL_LEFT + 300;
const LABEL_INSET: i32 = 32;

/// The body size, the same on every panel.
pub const FONT_PX: f32 = 32.0;

/// `grid::Layout`, in device pixels.
struct Layout {
    cols: usize,
    rows: usize,
    cell_h: u32,
    left: i32,
    top: i32,
}

impl Layout {
    fn compute(fb_xres: u32, fb_yres: u32) -> Self {
        let cols = ((fb_xres + COL_GAP) / (CELL_W + COL_GAP)).max(1) as usize;
        let avail = fb_yres.saturating_sub(HEADER_MARGIN + STRIP_H);
        let rows = ((avail + ROW_GAP) / (CELL_H_MIN + ROW_GAP)).max(1) as usize;
        let cell_h = (avail.saturating_sub((rows as u32 - 1) * ROW_GAP) / rows as u32)
            .clamp(CELL_H_MIN, CELL_H_MAX);
        let grid_w = cols as u32 * CELL_W + (cols as u32 - 1) * COL_GAP;
        let content_h = rows as u32 * cell_h + (rows as u32 - 1) * ROW_GAP;
        let slack = avail.saturating_sub(content_h);
        Self {
            cols,
            rows,
            cell_h,
            left: ((fb_xres as i32) - grid_w as i32) / 2,
            top: (HEADER_MARGIN + slack / 2) as i32,
        }
    }

    fn page_size(&self) -> usize {
        self.cols * self.rows
    }

    fn cell_xy(&self, idx: usize) -> (i32, i32) {
        let (col, row) = (idx % self.cols, idx / self.cols);
        (
            self.left + col as i32 * (CELL_W + COL_GAP) as i32,
            self.top + row as i32 * (self.cell_h + ROW_GAP) as i32,
        )
    }
}

/// One page of the grid, header band and toolbar included.
pub fn draw_page(fb: &mut Framebuffer, renderer: &mut TextRenderer, books: &[Book], page: usize) {
    let (xres, yres) = (fb.var.xres, fb.var.yres);
    fb.fill_rect(0, 0, xres, yres, 0xFF);

    let layout = Layout::compute(xres, yres);
    let pending = books.iter().filter(|b| !b.done).count();
    let total_pages = books.len().div_ceil(layout.page_size().max(1)).max(1);

    draw_header(fb, renderer, pending, books.len());

    let start = page * layout.page_size();
    let end = (start + layout.page_size()).min(books.len());
    for (slot, book) in books[start..end].iter().enumerate() {
        let (cx, cy) = layout.cell_xy(slot);
        draw_cell(fb, renderer, cx, cy, layout.cell_h, &book.title);
    }

    draw_strip(fb, renderer, page, total_pages, pending);
}

/// The band above the grid.
fn draw_header(fb: &mut Framebuffer, renderer: &mut TextRenderer, pending: usize, total: usize) {
    let xres = fb.var.xres;
    fb.fill_rect(HEADER_TOP, 0, xres, HEADER_H, 0xFF);
    let baseline = (HEADER_TOP + HEADER_H * 55 / 100) as i32;
    let count = match (pending, total) {
        (0, 0) => "No books found".to_string(),
        (0, _) => "All decrypted".to_string(),
        (1, _) => "1 book to decrypt".to_string(),
        (n, _) => format!("{n} books to decrypt"),
    };
    renderer.draw(fb, HEADER_MARGIN_X as i32, baseline, &count, false);

    let dest = "→ /mnt/us/dedrm";
    let w = renderer.measure_width(dest);
    let x = (xres.saturating_sub(HEADER_MARGIN_X) as i32 - w as i32).max(HEADER_MARGIN_X as i32);
    renderer.draw(fb, x, baseline, dest, false);
    fb.fill_rect(
        HEADER_TOP + HEADER_H - 2,
        HEADER_MARGIN_X,
        xres.saturating_sub(HEADER_MARGIN_X * 2),
        2,
        0x00,
    );
}

/// One cover cell: the placeholder fill and the title band.
fn draw_cell(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    cell_x: i32,
    cell_y: i32,
    cell_h: u32,
    title: &str,
) {
    if cell_x < 0 || cell_y < 0 {
        return;
    }
    fb.fill_rect(cell_y as u32, cell_x as u32, CELL_W, cell_h, 0xFF);
    let region_h = cell_h - NAME_BAND_H;
    fb.fill_rect(cell_y as u32, cell_x as u32, CELL_W, region_h, 0xDD);

    let band_top = cell_y as u32 + region_h;
    fb.fill_rect(band_top, cell_x as u32, CELL_W, 2, 0x00);
    let lines = renderer.wrap_and_clamp(title, CELL_W.saturating_sub(32), 1);
    if let Some(line) = lines.first() {
        let lw = renderer.measure_width(line);
        let lx = cell_x + ((CELL_W as i32 - lw as i32) / 2).max(0);
        let baseline = band_top as i32 + (NAME_BAND_H * 62 / 100) as i32;
        renderer.draw(fb, lx, baseline, line, false);
    }
}

/// The toolbar: fixed zones, a `Next` anchored to the screen's right edge, and
/// a separator at every zone edge whatever stands in the zone.
fn draw_strip(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    page: usize,
    total_pages: usize,
    pending: usize,
) {
    let strip_y = fb.var.yres.saturating_sub(STRIP_H);
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
    if page > 0 {
        renderer.draw(fb, NAV_LEFT as i32 + LABEL_INSET, baseline, "← Prev", false);
    }
    let mid_w = renderer.measure_width(&label_mid);
    let mid_x = (NAV_LEFT as i32 + fb.var.xres as i32) / 2 - mid_w as i32 / 2;
    renderer.draw(fb, mid_x, baseline, &label_mid, false);
    if page + 1 < total_pages {
        let next_w = renderer.measure_width("Next →");
        let next_x = fb.var.xres as i32 - LABEL_INSET * 2 - next_w as i32;
        renderer.draw(fb, next_x, baseline, "Next →", false);
    }
}
