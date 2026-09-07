//! Every screen drawn to a PNG, with no Kindle behind it.
//!
//! A shot names a screen and, after a colon, what it is showing: `grid:full`,
//! `toolbar:paged`. `--list` names them all, `--panel` picks the panel sizes,
//! and each shot is written once per panel.
//!
//! Drawing needs the device's faces, which a host does not have. Point
//! `KFXDEDRM_FONTS` at a directory holding them — a copy of the Kindle's
//! `/usr/java/lib/fonts` is what the shots are meant to look like.

mod fixture;

use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use kfxdedrm_fe_native::app;
use kfxdedrm_fe_native::eink::fb::Framebuffer;
use kfxdedrm_fe_native::engine::Missing;
use kfxdedrm_fe_native::ui::text::TextRenderer;
use kfxdedrm_fe_native::ui::{configmenu, grid, header, keyboard, pager, setup, toast};

/// The size every screen is drawn at, and the body size `app::run` loads.
const FONT_PX: f32 = 32.0;

/// Where the PNGs land under `--out`.
const OUT: &str = "artifacts/preview";

/// The panels, by the name `--panel` takes. Every width the X server reports
/// on a shipped device; the narrow two are where a layout runs out of room.
const PANELS: &[(&str, u32, u32)] = &[
    ("koa2", 1264, 1680),
    ("scribe", 1860, 2480),
    ("pw3", 1072, 1448),
    ("pw2", 758, 1024),
    ("basic", 600, 800),
];

/// What `--shot` takes, in the order `--list` prints them.
const SHOTS: &[&str] = &[
    "grid:full",
    "grid:one-page",
    "grid:empty",
    "grid:all-done",
    "toolbar:paged",
    "toolbar:one-page",
    "toolbar:nothing-pending",
    "settings",
    "settings:nothing-installed",
    "keyboard",
    "keyboard:collision",
    "setup:not-installed",
    "setup:no-working-build",
    "toast:hint",
    "toast:progress",
    "toast:progress-stop",
    "toast:done",
];

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--list") {
        for shot in SHOTS {
            println!("{shot}");
        }
        return Ok(());
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!(
            "preview [--shot NAME]... [--panel NAME]... [--out DIR] [--list]\n\
             \n\
             panels: {}\n\
             fonts:  KFXDEDRM_FONTS=/path/to/kindle/fonts",
            PANELS
                .iter()
                .map(|(n, ..)| *n)
                .collect::<Vec<_>>()
                .join(", ")
        );
        return Ok(());
    }

    let out = PathBuf::from(valued(&args, "--out").unwrap_or_else(|| OUT.to_string()));
    let shots = chosen(&args, "--shot", SHOTS)?;
    let names = chosen(
        &args,
        "--panel",
        &PANELS.iter().map(|(n, ..)| *n).collect::<Vec<_>>(),
    )?;
    let panels: Vec<&(&str, u32, u32)> = PANELS
        .iter()
        .filter(|(n, ..)| names.iter().any(|w| w == n))
        .collect();

    // `TextRenderer::load` reads the faces `font::discover` finds, which off a
    // Kindle is whatever `KFXDEDRM_FONTS` points at.
    let mut renderer = TextRenderer::load(FONT_PX)
        .context("no font — point KFXDEDRM_FONTS at a copy of the device's /usr/java/lib/fonts")?;
    eprintln!("fonts: {}", renderer.chain_description());

    std::fs::create_dir_all(&out).with_context(|| format!("create {}", out.display()))?;
    let mut written = 0usize;
    for (panel, xres, yres) in panels {
        for shot in &shots {
            let mut fb = Framebuffer::offscreen(*xres, *yres);
            render(&mut fb, &mut renderer, shot)?;
            let path = out.join(format!("{}-{panel}.png", shot.replace(':', "-")));
            fb.capture_png(&path)
                .with_context(|| format!("write {}", path.display()))?;
            println!("{}", path.display());
            written += 1;
        }
    }
    eprintln!("{written} shot(s) under {}", out.display());
    Ok(())
}

/// The value after `flag`, or `None`.
fn valued(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Every value given for `flag`, checked against `all`. No `flag` takes all of
/// them, which is the everyday run.
fn chosen(args: &[String], flag: &str, all: &[&str]) -> Result<Vec<String>> {
    let picked: Vec<String> = args
        .iter()
        .enumerate()
        .filter(|(_, a)| *a == flag)
        .filter_map(|(i, _)| args.get(i + 1).cloned())
        .collect();
    if picked.is_empty() {
        return Ok(all.iter().map(|s| s.to_string()).collect());
    }
    for want in &picked {
        if !all.iter().any(|a| a == want) {
            bail!("unknown {flag} {want:?} — one of: {}", all.join(", "));
        }
    }
    Ok(picked)
}

/// One shot into `fb`. Every arm paints the whole panel, so the caller hands
/// each a surface of its own.
fn render(fb: &mut Framebuffer, renderer: &mut TextRenderer, shot: &str) -> Result<()> {
    match shot {
        "grid:full" => {
            let books = fixture::books(11, false);
            grid_page(fb, renderer, &books, 0);
        }
        "grid:one-page" => {
            let books = fixture::books(3, false);
            grid_page(fb, renderer, &books, 0);
        }
        "grid:empty" => grid_page(fb, renderer, &[], 0),
        "grid:all-done" => {
            let books = fixture::books(6, true);
            grid_page(fb, renderer, &books, 0);
        }

        // The toolbar over a drawn grid, which is how it is ever seen.
        "toolbar:paged" => {
            let books = fixture::books(30, false);
            grid_page(fb, renderer, &books, 1);
        }
        "toolbar:one-page" => {
            let books = fixture::books(4, false);
            grid_page(fb, renderer, &books, 0);
        }
        "toolbar:nothing-pending" => {
            let books = fixture::books(4, true);
            grid_page(fb, renderer, &books, 0);
        }

        "settings" => {
            configmenu::draw(
                fb,
                renderer,
                &fixture::config(),
                &fixture::folders(),
                &fixture::installed(true),
            );
        }
        "settings:nothing-installed" => {
            configmenu::draw(
                fb,
                renderer,
                &fixture::config(),
                &fixture::folders(),
                &fixture::installed(false),
            );
        }

        "keyboard" => keyboard::draw(fb, renderer, "/mnt/us/dedrm", &fixture::config().scan_dirs),
        // A path already scanned, which the band refuses.
        "keyboard:collision" => keyboard::draw(
            fb,
            renderer,
            "/mnt/us/documents",
            &fixture::config().scan_dirs,
        ),

        "setup:not-installed" => setup::draw(fb, renderer, Missing::NotInstalled)?,
        "setup:no-working-build" => setup::draw(fb, renderer, Missing::NoWorkingBuild)?,

        // A banner reads against the screen it covers, so the grid goes down
        // first.
        "toast:hint" => {
            grid_page(fb, renderer, &fixture::books(6, false), 0);
            toast::draw(fb, renderer, "Hold to decrypt");
        }
        "toast:progress" => {
            grid_page(fb, renderer, &fixture::books(6, false), 0);
            toast::draw_progress(fb, renderer, "The Left Hand of Darkness", 2, 6);
        }
        "toast:progress-stop" => {
            grid_page(fb, renderer, &fixture::books(6, false), 0);
            toast::draw_progress_stop(fb, renderer, "The Left Hand of Darkness", 2, 6);
        }
        "toast:done" => {
            grid_page(fb, renderer, &fixture::books(6, false), 0);
            toast::draw_download_done(fb, renderer, "Decrypted 6");
        }

        other => bail!("unknown shot {other:?}"),
    }
    Ok(())
}

/// [`app::draw_page`] over `books`, with the covers left undrawn: a jacket is
/// a JPEG off the device and the layout is what these shots are for.
fn grid_page(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    books: &[kfxdedrm_fe_native::scan::Book],
    page: usize,
) {
    let layout = grid::Layout::compute(fb.var.xres, fb.var.yres, header::MARGIN, pager::STRIP_H);
    let covers = vec![None; books.len()];
    let total = pager::n_pages(books.len(), layout.page_size());
    app::draw_page(
        fb,
        renderer,
        books,
        &covers,
        &fixture::config(),
        layout,
        page.min(total.saturating_sub(1)),
        total,
    );
}
