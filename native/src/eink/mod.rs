//! On-device display + input plumbing.
//!
//! Display goes through a real X11 window (`fb.rs`, via the pure-Rust `x11rb`)
//! so the lab126 compositor manages + recomposites it.
//! Input is raw evdev (`touch.rs`, `buttons.rs`) with `EVIOCGRAB`, multiplexed
//! by `input.rs`. No `libxcb`/C dependencies.

pub mod buttons;
pub mod fb;
pub mod input;
pub mod screenshot;
pub mod touch;
pub mod xprobe;
