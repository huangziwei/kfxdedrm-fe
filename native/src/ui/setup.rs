//! The screen for a [`Missing`] from `engine::locate`.
//!
//! [`steps`] names `engine::RELEASE_ASSET`, `engine::RELEASES_URL` and
//! `engine::EXTENSION_DIR` on unwrapped lines: the panel has no browser and
//! the strings are transcribed by hand.
//!
//! One [`hit_exit`] zone. [`Missing`] survives a relaunch, leaving nothing for
//! a Retry to reach.

use crate::eink::fb::{Framebuffer, MxcfbRect, WAVEFORM_MODE_GC16};
use crate::eink::input::{Input, InputEvent};
use crate::eink::touch::TouchEvent;
use crate::engine::{self, Missing};
use crate::ui::text::TextRenderer;

/// Bottom button row, one [`hit_exit`] zone across its width.
const BTN_H: u32 = 120;
/// Left inset, and the per-side margin bounding `wrap_and_clamp`.
const MARGIN_X: u32 = 60;
/// Indent for [`steps`].
const STEP_INDENT: u32 = 40;

fn btn_top(yres: u32) -> u32 {
    yres.saturating_sub(BTN_H)
}

/// True on the Exit row. Everything above is dead space.
pub fn hit_exit(ty: u32, yres: u32) -> bool {
    ty >= btn_top(yres)
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
        String::new(),
        "3.  Eject the Kindle and start kfxdedrm-fe again".to_string(),
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

fn draw(fb: &mut Framebuffer, renderer: &mut TextRenderer, reason: Missing) -> anyhow::Result<()> {
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
         extension, and it has to be installed first.",
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

    draw_button(fb, renderer);

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

/// One full-width `[ Exit ]` row under a 2px rule.
fn draw_button(fb: &mut Framebuffer, renderer: &mut TextRenderer) {
    let xres = fb.var.xres;
    let top = btn_top(fb.var.yres);

    fb.fill_rect(top, 0, xres, 2, 0x00);
    fb.fill_rect(top + 2, 0, xres, BTN_H - 2, 0xFF);

    let label = "[ Exit ]";
    let w = renderer.measure_width(label);
    let x = ((xres as i32 - w as i32) / 2).max(0);
    renderer.draw(fb, x, (top + BTN_H * 60 / 100) as i32, label, false);
}

/// [`draw`], then block on [`hit_exit`].
pub fn run(
    fb: &mut Framebuffer,
    input: &mut Input,
    renderer: &mut TextRenderer,
    reason: Missing,
) -> anyhow::Result<()> {
    draw(fb, renderer, reason)?;
    loop {
        match input.next_event()? {
            InputEvent::Touch(TouchEvent::Up { y, .. }) if hit_exit(y, fb.var.yres) => {
                return Ok(());
            }
            InputEvent::Touch(TouchEvent::Screenshot) => {
                let _ = crate::eink::screenshot::capture(fb);
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
    fn only_the_bottom_row_exits() {
        let yres = 1448;
        assert!(hit_exit(yres - 1, yres));
        assert!(hit_exit(yres - BTN_H, yres));
        assert!(!hit_exit(yres - BTN_H - 1, yres));
        assert!(!hit_exit(0, yres));
    }
}
