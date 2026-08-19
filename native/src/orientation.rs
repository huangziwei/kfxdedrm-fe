//! Which way up the panel is.
//!
//! The KOA2 framework rotates the screen 180° by the accelerometer, and raw
//! evdev touch coordinates are panel-fixed. `Input::set_orientation` applies
//! the transform.
//!
//! `app::run` calls [`Orientation::detect`] at startup and on every
//! `InputEvent::Tick`, rebuilding `grid::Layout` on a change.

use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Native portrait, page-turn bezel right. Coordinates pass through.
    Up,
    /// Rotated 180°, page-turn bezel left. Both axes mirror.
    Down,
}

impl Orientation {
    /// `lipc-get-prop com.lab126.winmgr orientation`, which prints
    /// "U"/"D"/"L"/"R". Only U and D are distinguished. Any error or other
    /// output gives [`Orientation::Up`].
    pub fn detect() -> Self {
        let Ok(out) = Command::new("lipc-get-prop")
            .args(["com.lab126.winmgr", "orientation"])
            .output()
        else {
            return Self::Up;
        };
        if !out.status.success() {
            return Self::Up;
        }
        match String::from_utf8_lossy(&out.stdout).trim() {
            "D" => Self::Down,
            _ => Self::Up,
        }
    }
}
