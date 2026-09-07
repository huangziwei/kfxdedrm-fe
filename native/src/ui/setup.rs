//! The screen for a [`Missing`] from `engine::locate`, and the offer to fix it.
//!
//! [`steps`] names `engine::RELEASE_ASSET`, `engine::RELEASES_URL` and
//! `engine::EXTENSION_DIR` on unwrapped lines: the panel has no browser, and
//! whoever does this by hand transcribes them.
//!
//! Two zones on the bottom row. [`Choice::Install`] is `app::install_addons`,
//! which does over Wi-Fi exactly what [`steps`] describes; [`Choice::Skip`]
//! opens the app anyway. Neither ends the launch — nothing here decides
//! whether the app runs, only whether it fetches the engine first.

use crate::eink::fb::{Framebuffer, MxcfbRect, WAVEFORM_MODE_GC16};
use crate::eink::input::{Input, InputEvent};
use crate::eink::touch::TouchEvent;
use crate::engine::{self, Missing};
use crate::ui::strip;
use crate::ui::text::TextRenderer;

/// Bottom button row, split into the two [`Choice`] zones, on the shared
/// [`crate::ui::strip`].
const BTN_H: u32 = strip::H;
/// Left inset, and the per-side margin bounding `wrap_and_clamp`.
const MARGIN_X: u32 = 60;
/// Indent for [`steps`].
const STEP_INDENT: u32 = 40;

/// What the bottom row offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Fetch the engine and the add-on now.
    Install,
    /// Open the app without them.
    Skip,
}

fn btn_top(xres: u32, yres: u32) -> u32 {
    strip::top(xres, yres)
}

/// Where the two zones meet.
fn split(xres: u32) -> u32 {
    xres / 2
}

/// Which zone `(x, y)` fell in. Everything above the row is dead space.
pub fn hit(x: u32, y: u32, xres: u32, yres: u32) -> Option<Choice> {
    if y < btn_top(xres, yres) {
        return None;
    }
    Some(if x < split(xres) {
        Choice::Install
    } else {
        Choice::Skip
    })
}

/// One headline per [`Missing`]. [`Missing::NoWorkingBuild`] survives a repeat
/// of the same install.
fn headline(reason: Missing) -> &'static str {
    match reason {
        Missing::NotInstalled => "kfxdedrm is not installed",
        Missing::NoWorkingBuild => "No kfxdedrm build runs on this Kindle",
    }
}

/// What `engine::locate_in` found under `engine::BIN_DIR`.
fn subhead(reason: Missing) -> String {
    match reason {
        Missing::NotInstalled => {
            format!("Nothing found at {}", engine::BIN_DIR)
        }
        Missing::NoWorkingBuild => format!(
            "All {} builds in {} failed to start.",
            engine::ABI_VARIANTS.len(),
            engine::BIN_DIR
        ),
    }
}

/// Numbered steps. [`Missing::NoWorkingBuild`] opens on replacing the copy
/// under `engine::BIN_DIR`.
fn steps(reason: Missing) -> Vec<String> {
    let first = match reason {
        Missing::NotInstalled => "Download",
        Missing::NoWorkingBuild => "Re-download",
    };
    vec![
        format!("1.  {first}  {}", engine::RELEASE_ASSET),
        format!("     from  {}", engine::RELEASES_URL),
        String::new(),
        format!(
            "2.  Unzip it onto the Kindle as  {}/",
            engine::EXTENSION_DIR
        ),
    ]
}

/// One left-aligned line at `y`, advancing `y` by `lh`. Baseline 80% down.
fn draw_line(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    x: i32,
    y: &mut u32,
    lh: u32,
    s: &str,
) {
    if !s.is_empty() {
        let baseline = (*y + lh * 80 / 100) as i32;
        renderer.draw(fb, x, baseline, s, false);
    }
    *y += lh;
}

pub fn draw(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    reason: Missing,
) -> anyhow::Result<()> {
    fb.fill_rect(0, 0, fb.var.xres, fb.var.yres, 0xFF);

    let lh = renderer.line_height().max(1);
    let left = MARGIN_X as i32;
    let max_w = fb.var.xres.saturating_sub(MARGIN_X * 2);
    let mut y = lh * 2;

    draw_line(fb, renderer, left, &mut y, lh, headline(reason));
    y += lh / 2;
    for line in renderer.wrap_and_clamp(&subhead(reason), max_w, 2) {
        draw_line(fb, renderer, left, &mut y, lh, &line);
    }
    y += lh / 2;

    // Names the two extensions ahead of [`steps`].
    for line in renderer.wrap_and_clamp(
        "kfxdedrm-fe is a frontend. The engine that removes DRM is a separate \
         extension, and it is not part of this install.",
        max_w,
        3,
    ) {
        draw_line(fb, renderer, left, &mut y, lh, &line);
    }
    y += lh;

    // Unwrapped: a wrap splits the URL, the filename and the path.
    for step in steps(reason) {
        draw_line(fb, renderer, left + STEP_INDENT as i32, &mut y, lh, &step);
    }
    y += lh;

    for line in renderer.wrap_and_clamp(&offer(), max_w, 3) {
        draw_line(fb, renderer, left, &mut y, lh, &line);
    }

    draw_buttons(fb, renderer);

    fb.send_update(
        MxcfbRect {
            top: 0,
            left: 0,
            width: fb.var.xres,
            height: fb.var.yres,
        },
        WAVEFORM_MODE_GC16,
    )?;
    Ok(())
}

/// What the two buttons do, said once so the labels can stay two words.
fn offer() -> String {
    format!(
        "Install does both over Wi-Fi, and fetches the optional {} add-on \
         with it. Skip opens the app without them — Settings can fetch them \
         later.",
        crate::convert::EXTENSION_DIR
            .rsplit('/')
            .next()
            .unwrap_or("bokai")
    )
}

/// The bottom row: [`Choice::Install`] on the left, [`Choice::Skip`] on the
/// right, under a 2px rule and either side of a divider.
fn draw_buttons(fb: &mut Framebuffer, renderer: &mut TextRenderer) {
    let xres = fb.var.xres;
    let top = strip::base(fb);
    let baseline = strip::baseline(xres, top, renderer);
    let mid = split(xres);
    strip::separator(fb, mid);
    strip::label(fb, renderer, 0, mid, baseline, "[ Install ]", false);
    strip::label(fb, renderer, mid, xres - mid, baseline, "[ Skip ]", false);
}

/// [`draw`], then block on the bottom row.
pub fn run(
    fb: &mut Framebuffer,
    input: &mut Input,
    renderer: &mut TextRenderer,
    reason: Missing,
) -> anyhow::Result<Choice> {
    draw(fb, renderer, reason)?;
    loop {
        match input.next_event()? {
            InputEvent::Touch(TouchEvent::Up { x, y }) => {
                if let Some(choice) = hit(x, y, fb.var.xres, fb.var.yres) {
                    return Ok(choice);
                }
            }
            InputEvent::Touch(TouchEvent::Screenshot) => {
                let _ = crate::eink::screenshot::capture(fb);
            }
            InputEvent::Tick => {
                // This is the first screen the app draws and it blocks until
                // answered, so a cover or a rotation here is followed nowhere
                // else.
                let pump = fb.pump_events();
                if let Some(covered) = pump.covered {
                    input.set_covered(covered);
                }
                input.retake();
                if pump.resized.is_some() {
                    input.set_size(fb.var.xres, fb.var.yres);
                }
                let turned = input.follow_orientation();
                let redraw = match pump.covered {
                    // Covered: the panel belongs to the window over this one.
                    Some(true) => false,
                    // Uncovered: drawn without an `Expose` to ask for it.
                    Some(false) => true,
                    None => turned || pump.resized.is_some() || pump.repaint,
                };
                if redraw {
                    draw(fb, renderer, reason)?;
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

    #[test]
    fn the_two_failures_do_not_share_their_wording() {
        assert_ne!(
            headline(Missing::NotInstalled),
            headline(Missing::NoWorkingBuild)
        );
        assert_ne!(
            subhead(Missing::NotInstalled),
            subhead(Missing::NoWorkingBuild)
        );
        // [`Missing::NoWorkingBuild`] opens on replacing the install.
        assert!(steps(Missing::NoWorkingBuild)[0].contains("Re-download"));
        assert!(!steps(Missing::NotInstalled)[0].contains("Re-download"));
    }

    #[test]
    fn the_steps_name_the_asset_the_url_and_the_destination() {
        // [`steps`] is the one place these three strings appear together.
        let text = steps(Missing::NotInstalled).join("\n");
        assert!(text.contains(engine::RELEASE_ASSET));
        assert!(text.contains(engine::RELEASES_URL));
        assert!(text.contains(engine::EXTENSION_DIR));
    }

    #[test]
    fn the_bottom_row_is_the_only_thing_that_takes_a_tap() {
        let (xres, yres) = (1072u32, 1448u32);
        assert_eq!(hit(10, yres - 1, xres, yres), Some(Choice::Install));
        assert_eq!(hit(10, yres - BTN_H, xres, yres), Some(Choice::Install));
        assert_eq!(hit(xres - 10, yres - 1, xres, yres), Some(Choice::Skip));
        // The row is split down the middle and nothing lands between.
        assert_eq!(
            hit(xres / 2 - 1, yres - 10, xres, yres),
            Some(Choice::Install)
        );
        assert_eq!(hit(xres / 2, yres - 10, xres, yres), Some(Choice::Skip));
        // Everything above it is dead space.
        assert_eq!(hit(10, yres - BTN_H - 1, xres, yres), None);
        assert_eq!(hit(10, 0, xres, yres), None);
    }

    #[test]
    fn the_offer_says_what_each_button_does() {
        let offer = offer();
        assert!(offer.contains("Install"));
        assert!(offer.contains("Skip"));
        // The add-on it fetches alongside the engine is named.
        assert!(offer.contains("bokai"), "{offer}");
    }
}
