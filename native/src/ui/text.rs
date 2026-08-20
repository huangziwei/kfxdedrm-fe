//! Text rasterization onto the framebuffer.
//!
//! Glyphs come from a fallback chain over the faces the device already ships
//! (see [`crate::font`]), outlined one at a time at a fixed size via
//! ab_glyph. The rasterizer reports per-pixel coverage; we threshold it to
//! 1-bit because eink's DU waveform is B/W and antialiased gray smears on the
//! panel. Coverage above 96/255 becomes a black pixel, below stays white.
//! Crisp at small sizes; would need GC16 + dithering for true grayscale text.
//!
//! A character no face in the chain has draws a deliberate hollow box.
//! Handing it to the rasterizer instead blits the font's own `.notdef`,
//! whose hairline outline mostly falls *under* the coverage threshold — what
//! reaches the panel is a bar and two stray dots, which reads as data
//! corruption rather than as a missing glyph.
//!
//! Glyphs are cached per (codepoint, px, face) because rasterization isn't
//! free and CJK titles repeat characters often. The face belongs in the key:
//! two faces rasterize the same codepoint to different shapes.

use std::collections::HashMap;

use ab_glyph::{Font as _, FontVec, ScaleFont as _};
use anyhow::Result;

use crate::eink::fb::Framebuffer;
use crate::font::{self, FontChain};
use crate::ui::{BLACK, WHITE};

const COVERAGE_THRESHOLD: u8 = 96;

/// One rasterized glyph: where it sits relative to the pen, and its coverage.
///
/// Offsets are what the blit needs and nothing more — `left` from the pen's x,
/// `top` from the baseline and growing *downward*, matching the rasterizer's
/// screen-space convention. A glyph with no outline (a space) keeps its
/// advance and carries an empty bitmap.
struct Raster {
    advance: f32,
    left: i32,
    top: i32,
    width: usize,
    height: usize,
    coverage: Vec<u8>,
}

pub struct TextRenderer {
    chain: FontChain,
    px: f32,
    cache: HashMap<(char, u32, usize), Raster>,
}

impl TextRenderer {
    pub fn load(px: f32) -> Result<Self> {
        Ok(Self {
            chain: FontChain::load(&font::discover())?,
            px,
            cache: HashMap::new(),
        })
    }

    /// The fallback chain this device ended up with, primary first, for the
    /// startup log — see [`FontChain::paths`].
    pub fn chain_description(&self) -> String {
        self.chain
            .paths()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(" -> ")
    }

    /// Run `f` with the renderer at `px`, restoring the previous size after.
    ///
    /// One chain and one copy of every face however many sizes are drawn: the
    /// glyph cache is keyed by size, so a second size costs the glyphs it
    /// actually draws and nothing else. A second [`TextRenderer`] would read
    /// the face files again, which for a CJK face is tens of megabytes.
    ///
    /// [`TextRenderer::line_height`] and [`TextRenderer::measure_width`] read
    /// the size too, so both answer for `px` inside `f`.
    pub fn at_px<R>(&mut self, px: f32, f: impl FnOnce(&mut Self) -> R) -> R {
        let previous = std::mem::replace(&mut self.px, px);
        let out = f(self);
        self.px = previous;
        out
    }

    pub fn line_height(&self) -> u32 {
        // The face's own vertical metrics; round up so adjacent rows don't
        // tear into each other. Always the primary face's, so a row keeps its
        // height whichever face draws the text.
        let face = self.chain.primary().as_scaled(self.px);
        (face.height() + face.line_gap()).ceil().max(1.0) as u32
    }

    /// Total advance width of `s` at the current px. Used by the overlay
    /// to center text inside the banner.
    ///
    /// Resolves faces exactly the way [`TextRenderer::draw`] does, over the
    /// same string, so a measured width is the width that gets drawn.
    pub fn measure_width(&mut self, s: &str) -> u32 {
        self.measure_width_in(font::Script::Unknown, s)
    }

    /// [`TextRenderer::measure_width`] for text whose language is known — see
    /// [`TextRenderer::draw_in`].
    pub fn measure_width_in(&mut self, script: font::Script, s: &str) -> u32 {
        let selection = self.chain.select(s, script);
        let px = self.px;
        let px_key = px.to_bits();
        let mut w = 0u32;
        for ch in s.chars() {
            if font::is_invisible(ch) {
                continue;
            }
            let advance = match self.chain.glyph_source(selection, ch) {
                Some((face, font)) => {
                    let entry = self
                        .cache
                        .entry((ch, px_key, face))
                        .or_insert_with(|| rasterize(font, ch, px));
                    entry.advance.round().max(0.0) as u32
                }
                None => missing_advance(px),
            };
            w = w.saturating_add(advance);
        }
        w
    }

    /// Word-wrap `text` to fit `max_width` per line, then clamp to at most
    /// `max_lines`, ellipsizing the dropped tail. Latin titles wrap at
    /// whitespace; CJK titles (no spaces) fall through to char-level wrap
    /// so they pack densely without overflowing the box. Thin font-backed
    /// wrapper over [`crate::wrap::wrap_and_clamp`]; shared by the cover
    /// placeholder and [`crate::ui::setup`].
    pub fn wrap_and_clamp(&mut self, text: &str, max_width: u32, max_lines: usize) -> Vec<String> {
        self.wrap_and_clamp_in(font::Script::Unknown, text, max_width, max_lines)
    }

    /// [`TextRenderer::wrap_and_clamp`] for text whose language is known — see
    /// [`TextRenderer::draw_in`].
    pub fn wrap_and_clamp_in(
        &mut self,
        script: font::Script,
        text: &str,
        max_width: u32,
        max_lines: usize,
    ) -> Vec<String> {
        crate::wrap::wrap_and_clamp(text, max_width, max_lines, |s| {
            self.measure_width_in(script, s)
        })
    }
}

impl TextRenderer {
    /// Draw `s` starting at baseline (x, y_baseline). Returns the
    /// advanced X. `inverted=true` swaps colors (white-on-black) so the
    /// caller can highlight a tapped row by painting the row's background
    /// black first and calling with `inverted=true`.
    pub fn draw(
        &mut self,
        fb: &mut Framebuffer,
        x: i32,
        y_baseline: i32,
        s: &str,
        inverted: bool,
    ) -> i32 {
        self.draw_in(font::Script::Unknown, fb, x, y_baseline, s, inverted)
    }

    /// [`TextRenderer::draw`] in an explicit shade — `ui::QUIET` for a line
    /// that is not a control, or either of the two `draw` picks between.
    pub fn draw_ink(
        &mut self,
        fb: &mut Framebuffer,
        x: i32,
        y_baseline: i32,
        s: &str,
        ink: u8,
    ) -> i32 {
        self.draw_in_ink(font::Script::Unknown, fb, x, y_baseline, s, ink)
    }

    /// [`TextRenderer::draw_ink`] struck twice, a pixel apart.
    ///
    /// `font::rank` puts upright regulars first and a device may carry no bold
    /// cut at all, so a heading is thickened rather than set in another face.
    /// The second strike widens the run by one pixel, which
    /// [`TextRenderer::measure_width`] does not account for.
    pub fn draw_bold(
        &mut self,
        fb: &mut Framebuffer,
        x: i32,
        y_baseline: i32,
        s: &str,
        ink: u8,
    ) -> i32 {
        self.draw_ink(fb, x, y_baseline, s, ink);
        self.draw_ink(fb, x + 1, y_baseline, s, ink)
    }

    /// [`TextRenderer::draw`] for text whose language is known — a book title
    /// from a tagged book, rather than the app's own chrome.
    ///
    /// The hint decides which face is *tried* first, not which one draws:
    /// coverage still has the last word, so a book tagged with the wrong
    /// language gets the wrong regional shapes but never a missing glyph.
    /// Without it, a Traditional Chinese title silently keeps the Japanese
    /// face — which covers it, so nothing looks broken enough to notice.
    pub fn draw_in(
        &mut self,
        script: font::Script,
        fb: &mut Framebuffer,
        x: i32,
        y_baseline: i32,
        s: &str,
        inverted: bool,
    ) -> i32 {
        let ink = if inverted { WHITE } else { BLACK };
        self.draw_in_ink(script, fb, x, y_baseline, s, ink)
    }

    /// [`TextRenderer::draw_in`] in an explicit shade.
    pub fn draw_in_ink(
        &mut self,
        script: font::Script,
        fb: &mut Framebuffer,
        x: i32,
        y_baseline: i32,
        s: &str,
        fg: u8,
    ) -> i32 {
        let selection = self.chain.select(s, script);
        let px = self.px;
        let px_key = px.to_bits();
        let mut cur_x = x;
        for ch in s.chars() {
            if font::is_invisible(ch) {
                continue;
            }
            match self.chain.glyph_source(selection, ch) {
                // Cache key uses bit pattern of f32 — same px always keys the same.
                Some((face, font)) => {
                    let glyph = self
                        .cache
                        .entry((ch, px_key, face))
                        .or_insert_with(|| rasterize(font, ch, px));
                    blit_threshold(
                        fb,
                        cur_x + glyph.left,
                        y_baseline + glyph.top,
                        glyph.width,
                        glyph.height,
                        &glyph.coverage,
                        fg,
                    );
                    cur_x += glyph.advance.round() as i32;
                }
                None => {
                    draw_missing(fb, cur_x, y_baseline, px, fg);
                    cur_x += missing_advance(px) as i32;
                }
            }
        }
        cur_x
    }
}

/// Outline `ch` from `font` at `px` and collect its coverage.
///
/// The rasterizer works in screen space — y grows downward from the baseline —
/// so the bounds it reports are already the offsets the blit wants. A
/// character with no outline at all (a space, or a glyph defined as blank)
/// still has an advance, and returns an empty bitmap rather than nothing.
fn rasterize(font: &FontVec, ch: char, px: f32) -> Raster {
    let id = font.glyph_id(ch);
    let advance = font.as_scaled(px).h_advance(id);
    let Some(outline) = font.outline_glyph(id.with_scale(px)) else {
        return Raster {
            advance,
            left: 0,
            top: 0,
            width: 0,
            height: 0,
            coverage: Vec::new(),
        };
    };
    // Bounds are already whole pixels (floored/ceiled), and the rasterizer
    // sizes its grid with this same expression — matching it keeps the buffer
    // exactly the extent `draw` emits into.
    let bounds = outline.px_bounds();
    let (width, height) = (bounds.width() as usize, bounds.height() as usize);
    let mut coverage = vec![0u8; width * height];
    outline.draw(|x, y, c| {
        let (x, y) = (x as usize, y as usize);
        if x < width && y < height {
            coverage[y * width + x] = (c * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    });
    Raster {
        advance,
        left: bounds.min.x.round() as i32,
        top: bounds.min.y.round() as i32,
        width,
        height,
        coverage,
    }
}

/// Advance of the missing-glyph mark: an ideograph's share of the line, so a
/// run of unmappable characters keeps the text's rhythm. Shared by
/// [`TextRenderer::measure_width`] and [`TextRenderer::draw`] so a line is
/// measured at the width it will be drawn at.
fn missing_advance(px: f32) -> u32 {
    (px * 0.72).round().max(6.0) as u32
}

/// A hollow box standing on the baseline, for a character no face in the
/// chain has. Stroked 2px on purpose: a hairline outline is exactly what
/// makes a font's own `.notdef` fall apart under [`COVERAGE_THRESHOLD`].
fn draw_missing(fb: &mut Framebuffer, x: i32, y_baseline: i32, px: f32, fg: u8) {
    const STROKE: i32 = 2;
    let (left, right) = (x + STROKE, x + missing_advance(px) as i32 - STROKE * 2);
    let (top, bottom) = (y_baseline - (px * 0.66).round() as i32, y_baseline - STROKE);
    if right - left < STROKE * 2 || bottom - top < STROKE * 2 {
        return;
    }
    for row in top..=bottom {
        let horizontal_edge = row < top + STROKE || row > bottom - STROKE;
        for col in left..=right {
            let vertical_edge = col < left + STROKE || col > right - STROKE;
            if horizontal_edge || vertical_edge {
                fb.put_pixel(col, row, fg);
            }
        }
    }
}

fn blit_threshold(
    fb: &mut Framebuffer,
    x: i32,
    y: i32,
    w: usize,
    h: usize,
    coverage: &[u8],
    fg: u8,
) {
    if w == 0 || h == 0 {
        return;
    }
    // put_pixel applies the orientation transform + bounds check. Glyphs
    // are small (≤32x32 typically), so per-pixel call overhead is fine.
    for row in 0..h {
        let cov_row = &coverage[row * w..row * w + w];
        for (col, &cov) in cov_row.iter().enumerate() {
            if cov >= COVERAGE_THRESHOLD {
                fb.put_pixel(x + col as i32, y + row as i32, fg);
            }
        }
    }
}
