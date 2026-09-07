//! Physical sizing.
//!
//! Every pixel constant in `ui` is a *design pixel*, written for a
//! [`DESIGN_DPI`] panel; [`Scale::px`] turns one into the device pixels that
//! draw it at the same physical size anywhere else. A tap target is a
//! fingertip and a body line is a reading size, so a constant that reaches the
//! panel unscaled is 42% too large at 212 dpi and 80% too large at 167.
//!
//! The density is read off the panel width, because that is what the X server
//! reports and nothing on the device names the model. Every width the shipped
//! firmware reports is in [`PANELS`], and an unknown one takes
//! [`DESIGN_DPI`] — the density of every Kindle from the Voyage on.

/// The density every `ui` constant is written at: 300 ppi, the Voyage and
/// everything after it.
pub const DESIGN_DPI: u32 = 300;

/// Panel density by framebuffer width, for the widths that are not
/// [`DESIGN_DPI`].
///
/// | width | device | ppi |
/// |:--|:--|:--|
/// | 600 | 6" Kindle Touch, Basic 1–2 | 167 |
/// | 758 | 6" Paperwhite 1–2 | 212 |
///
/// The rest are 300: 1072 (Voyage, Paperwhite 3–4, Basic 4), 1236
/// (Paperwhite 5–6, Colorsoft), 1264 (Oasis 2–3), 1860 (Scribe).
const PANELS: &[(u32, u32)] = &[(600, 167), (758, 212)];

/// What one design pixel is worth on the panel being drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scale {
    dpi: u32,
}

impl Scale {
    /// The density [`PANELS`] gives `fb_xres`, else [`DESIGN_DPI`].
    pub fn of_width(fb_xres: u32) -> Self {
        let dpi = PANELS
            .iter()
            .find(|(width, _)| *width == fb_xres)
            .map(|(_, dpi)| *dpi)
            .unwrap_or(DESIGN_DPI);
        Self { dpi }
    }

    /// A scale that leaves every constant alone, for a caller measuring in
    /// device pixels already.
    pub fn none() -> Self {
        Self { dpi: DESIGN_DPI }
    }

    pub fn dpi(&self) -> u32 {
        self.dpi
    }

    /// `design` in device pixels. A non-zero constant never scales to 0: a
    /// hairline that rounds away leaves the shape it divides unreadable.
    pub fn px(&self, design: u32) -> u32 {
        match design {
            0 => 0,
            _ => (design * self.dpi / DESIGN_DPI).max(1),
        }
    }

    /// [`Scale::px`] for a signed constant.
    pub fn i(&self, design: i32) -> i32 {
        let scaled = self.px(design.unsigned_abs()) as i32;
        match design < 0 {
            true => -scaled,
            false => scaled,
        }
    }

    /// [`Scale::px`] for a font size, which keeps its fraction.
    pub fn font(&self, design: f32) -> f32 {
        design * self.dpi as f32 / DESIGN_DPI as f32
    }
}

#[cfg(test)]
mod tests {
    use super::{DESIGN_DPI, Scale};

    /// The two panels that are not [`DESIGN_DPI`], and the ones that are.
    #[test]
    fn the_shipped_widths_carry_their_own_density() {
        assert_eq!(Scale::of_width(600).dpi(), 167);
        assert_eq!(Scale::of_width(758).dpi(), 212);
        for wide in [1072, 1236, 1264, 1860] {
            assert_eq!(Scale::of_width(wide).dpi(), DESIGN_DPI, "{wide}");
        }
        // An unreported width draws at the density most devices have.
        assert_eq!(Scale::of_width(999).dpi(), DESIGN_DPI);
    }

    /// A design pixel is itself at [`DESIGN_DPI`] and smaller below it.
    #[test]
    fn a_design_pixel_shrinks_with_the_panel() {
        let full = Scale::of_width(1264);
        assert_eq!(full.px(360), 360);
        assert_eq!(full.font(32.0), 32.0);

        let pw2 = Scale::of_width(758);
        assert_eq!(pw2.px(360), 360 * 212 / 300);
        assert_eq!(pw2.px(80), 56);

        let basic = Scale::of_width(600);
        assert_eq!(basic.px(360), 360 * 167 / 300);
    }

    /// A constant that would round to nothing keeps one pixel, and a zero
    /// stays zero.
    #[test]
    fn a_hairline_survives_the_smallest_panel() {
        let basic = Scale::of_width(600);
        assert_eq!(basic.px(1), 1);
        assert_eq!(basic.px(2), 1);
        assert_eq!(basic.px(0), 0);
    }

    /// [`Scale::i`] keeps the sign.
    #[test]
    fn a_signed_constant_scales_both_ways() {
        let pw2 = Scale::of_width(758);
        assert_eq!(pw2.i(44), pw2.px(44) as i32);
        assert_eq!(pw2.i(-44), -(pw2.px(44) as i32));
        assert_eq!(pw2.i(0), 0);
    }
}
