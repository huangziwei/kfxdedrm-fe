//! Text rasterization, the cover grid, the settings page, and the blocking
//! overlays.
//!
//! Every pixel constant here is a design pixel at [`scale::DESIGN_DPI`]; the
//! module that draws with it maps it through [`scale::Scale`] for the panel in
//! front of it.

pub mod configmenu;
pub mod grid;
pub mod header;
pub mod keyboard;
pub mod pager;
pub mod panel;
pub mod pathbar;
pub mod scale;
pub mod setup;
pub mod strip;
pub mod text;
pub mod toast;

/// Shades everything here draws in. `eink::fb` stores one byte per channel and
/// `ui::text` stamps a glyph in one of these.
pub const BLACK: u8 = 0x00;
pub const WHITE: u8 = 0xFF;
/// A line on the page to be read, not acted on: the note under a row, and a
/// chip naming where a setting stands without offering to change it.
///
/// `WAVEFORM_MODE_GC16` resolves 16 levels and holds this shade.
/// `WAVEFORM_MODE_DU` is two-level and snaps it: nothing in this shade may sit
/// inside a DU rect.
pub const QUIET: u8 = 0x88;
