//! kfxdedrm-fe: a frontend for the kfxdedrm engine on a jailbroken Kindle.
//!
//! - [`config`], [`engine`], [`mobi`], [`scan`] — which books are listed, which
//!   engine build runs, what it receives, where its output lands.
//! - [`convert`] — the optional bokai add-on, and the extra formats the
//!   settings ask it for beside that output.
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
pub mod mobi;
pub mod orientation;
pub mod scan;
pub mod ui;
pub mod wrap;
