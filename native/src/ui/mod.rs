//! Text rasterization, the cover grid, the settings page, and the blocking
//! overlays.

pub mod configmenu;
pub mod grid;
pub mod header;
pub mod pager;
pub mod panel;
pub mod setup;
pub mod text;
pub mod toast;

/// Shades everything here draws in. `eink::fb` stores one byte per channel and
/// `ui::text` stamps a glyph in one of these.
pub const BLACK: u8 = 0x00;
pub const WHITE: u8 = 0xFF;
/// A line that is on the page to be read rather than acted on: the note under
/// a row, and a chip that names where a setting stands without offering to
/// change it.
///
/// Mid-grey survives a `WAVEFORM_MODE_GC16` refresh, which resolves 16 levels.
/// A `WAVEFORM_MODE_DU` region is two-level and will snap it, so nothing drawn
/// in this shade may sit inside a DU rect.
pub const QUIET: u8 = 0x88;
