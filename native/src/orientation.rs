//! Which way up the panel is.
//!
//! The KOA2 framework rotates the screen 180° by the accelerometer, and raw
//! evdev touch coordinates are panel-fixed. `Input::set_orientation` applies
//! the transform.
//!
//! `Input::follow_orientation` polls [`Orientation::detect`] and applies a
//! change to both devices; the panel loops rebuild their layout on the `true`
//! it answers.

use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Native portrait, page-turn bezel right. Coordinates pass through.
    Up,
    /// Rotated 180°, page-turn bezel left. Both axes mirror.
    Down,
}

impl Orientation {
    /// `lipc-get-prop com.lab126.winmgr orientation`, through
    /// [`Orientation::of_prop`]. An error reads as [`Orientation::Up`].
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
        Self::of_prop(&String::from_utf8_lossy(&out.stdout))
    }

    /// The `com.lab126.winmgr orientation` value: "U", "D", "L" or "R". "D" is
    /// [`Orientation::Down`] and every other value [`Orientation::Up`]; the two
    /// landscape values change the panel size, which `fb::Framebuffer` follows
    /// through a `ConfigureNotify` rather than here.
    pub fn of_prop(said: &str) -> Self {
        match said.trim() {
            "D" => Self::Down,
            _ => Self::Up,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Orientation;

    /// "D" alone is [`Orientation::Down`], through the trailing newline
    /// `lipc-get-prop` prints.
    #[test]
    fn only_d_is_down() {
        assert_eq!(Orientation::of_prop("D\n"), Orientation::Down);
        for said in ["U\n", "U", "", "  ", "d", "Down"] {
            assert_eq!(Orientation::of_prop(said), Orientation::Up, "{said:?}");
        }
    }

    /// "L" and "R" read as [`Orientation::Up`].
    #[test]
    fn landscape_reads_as_up() {
        assert_eq!(Orientation::of_prop("L"), Orientation::Up);
        assert_eq!(Orientation::of_prop("R"), Orientation::Up);
    }
}
