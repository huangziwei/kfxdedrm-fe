//! kfxdedrm-fe: a frontend for the kfxdedrm engine on a jailbroken Kindle.
//!
//! - [`config`], [`engine`], [`mobi`], [`scan`] — which books are listed, which
//!   engine build runs, what it receives, where its output lands.
//! - [`convert`] — the optional bokai add-on, and the extra formats the
//!   settings ask it for beside that output.
//! - [`install`], [`net`] — fetching the engine and the add-on from their own
//!   GitHub releases, and whether there is a route to do it over.
//! - [`eink`] — the framebuffer window, the evdev touchscreen and bezel keys.
//! - [`ui`], [`font`], [`wrap`] — what is drawn.
//! - [`app`] — the run loop.
//!
//! `main.rs` calls [`app::run`]. Every module builds on a host, so
//! `cargo test` covers all of them.

// `eink` and `ui` carry helpers [`app`] does not call: some input polling, a
// few geometry accessors, `ui::grid::draw_series_cell`.
#![allow(dead_code)]

pub mod app;
pub mod config;
pub mod convert;
pub mod eink;
pub mod engine;
pub mod font;
pub mod install;
pub mod mobi;
pub mod net;
pub mod orientation;
pub mod scan;
pub mod ui;
pub mod wrap;

/// One line to stderr. `launch.sh` redirects that to
/// `/mnt/us/logs/kfxdedrm-fe.log`; opening that file here too doubles every
/// line.
pub fn log(msg: impl AsRef<str>) {
    eprintln!("[{}] {}", now(), msg.as_ref());
}

fn now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "?".into())
}
