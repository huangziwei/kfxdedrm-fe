//! The two-corner screenshot gesture.
//!
//! [`super::touch`] holds an exclusive `EVIOCGRAB`, leaving the firmware's own
//! recognizer no touch events to read. `touch.rs` recognizes the gesture and
//! [`capture`] encodes `Framebuffer`'s backing buffer.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use super::fb::{Framebuffer, MxcfbRect, WAVEFORM_MODE_GC16};

/// Where stock Kindle screenshots land.
const SCREENSHOT_DIR: &str = "/mnt/us/screenshots";

/// White-flash hold.
const FLASH_MS: u64 = 120;

/// The screen to a timestamped PNG, a white flash, then the screen back.
///
/// No rotation: the backing holds the upright UI and the compositor rotates
/// the display. See [`Framebuffer::capture_png`].
///
/// The flash and the restore run whatever the encode returned.
pub fn capture(fb: &mut Framebuffer) -> Result<PathBuf> {
    let dir = Path::new(SCREENSHOT_DIR);
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("screenshot_{secs}.png"));

    // `capture_png` reads the live screen; `restore_backing` takes this back.
    let snap = fb.backing_snapshot();
    let cap = fb.capture_png(&path);

    // `send_update` widens to full rows; GC16 is the full refresh.
    let (w, h) = (fb.var.xres, fb.var.yres);
    let full = MxcfbRect {
        top: 0,
        left: 0,
        width: w,
        height: h,
    };
    fb.fill_rect(0, 0, w, h, 0xFF);
    let _ = fb.send_update(full, WAVEFORM_MODE_GC16);
    std::thread::sleep(Duration::from_millis(FLASH_MS));
    fb.restore_backing(snap);
    let _ = fb.send_update(full, WAVEFORM_MODE_GC16);

    cap.map(|()| path)
}
