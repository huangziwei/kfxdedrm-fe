//! [`run`]: a paginated grid of `[crate::scan::Book]`, with [`ui::configmenu`]
//! as a blocking overlay. A held cover runs [`decrypt_one`]; the toolbar runs
//! [`decrypt_all`].
//!
//! [`engine::locate`] failing ends the launch at [`ui::setup`].
//!
//! No path under `[config::DOCUMENTS_DIR]` is written, moved or removed.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use image::DynamicImage;

use crate::config::{self, Config};
use crate::eink;
use crate::eink::buttons::{Buttons, PageButton};
use crate::eink::fb::{Framebuffer, MxcfbRect, WAVEFORM_MODE_DU, WAVEFORM_MODE_GC16};
use crate::eink::input::{Input, InputEvent};
use crate::eink::touch::{Touch, TouchEvent};
use crate::engine::{self, Engine};
use crate::font;
use crate::orientation::Orientation;
use crate::scan::{self, Book};
use crate::ui::text::TextRenderer;
use crate::ui::{configmenu, grid, header, pager, setup, toast};

/// Root holding [`config_path`].
const BUNDLE_DIR: &str = "/mnt/us/extensions/kfxdedrm-fe";
/// `launch.sh` appends this process's stderr here.
const LOG_PATH: &str = "/mnt/us/logs/kfxdedrm-fe.log";

const FONT_PX: f32 = 32.0;

/// Hold on a cover, within [`ARM_SLOP_PX`] of the landing point, that runs
/// [`decrypt_one`]. A shorter press does nothing.
const ARM_THRESHOLD: Duration = Duration::from_millis(1000);
/// Drift on either axis, in user-visible px, that cancels an [`Armed`].
const ARM_SLOP_PX: u32 = 40;
/// `grid::draw_arm_cue` holds the panel before the first banner paints.
const ARM_DWELL: Duration = Duration::from_millis(250);
/// Result banner time before `repaint!`.
const RESULT_LINGER: Duration = Duration::from_millis(1100);
/// Hint banner time after a release short of [`ARM_THRESHOLD`].
const HINT_LINGER: Duration = Duration::from_millis(1200);

/// Floor on `toast::draw` repaints in [`decrypt_one`]. Each is a full-panel
/// GC16.
const TOAST_REDRAW_INTERVAL: Duration = Duration::from_millis(700);
/// Gap between `input` polls while the engine runs. A gesture wakes the poll
/// immediately; this bounds how late an exit is noticed.
const ENGINE_POLL: Duration = Duration::from_millis(250);

/// The settings file, beside the binary.
fn config_path() -> PathBuf {
    Path::new(BUNDLE_DIR).join("config.txt")
}

/// One line to stderr. `launch.sh` redirects that to [`LOG_PATH`]; opening
/// [`LOG_PATH`] here too doubles every line.
fn log(msg: impl AsRef<str>) {
    eprintln!("[{}] {}", now(), msg.as_ref());
}

fn now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "?".into())
}

fn full_rect(fb: &Framebuffer) -> MxcfbRect {
    MxcfbRect {
        top: 0,
        left: 0,
        width: fb.var.xres,
        height: fb.var.yres,
    }
}

/// Refresh rect for one grid cell.
fn cell_rect(cell_x: i32, cell_y: i32, cell_h: u32) -> MxcfbRect {
    MxcfbRect {
        top: cell_y.max(0) as u32,
        left: cell_x.max(0) as u32,
        width: grid::CELL_W,
        height: cell_h,
    }
}

/// A cover under a finger, pending [`ARM_THRESHOLD`]. An earlier release
/// draws the hold hint.
struct Armed {
    /// Cell index on the current page.
    slot: usize,
    /// Index into `books`.
    idx: usize,
    down_at: Instant,
    /// Landing point, against [`ARM_SLOP_PX`].
    at: (u32, u32),
}

/// `title` clipped to `max` chars, ellipsized.
fn short_title(title: &str, max: usize) -> String {
    if title.chars().count() <= max {
        return title.to_string();
    }
    let kept: String = title.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

fn label_of(book: &Book) -> grid::Label<'_> {
    grid::Label {
        text: &book.title,
        // `Book::title` comes from a filename and carries no language tag.
        script: font::Script::Unknown,
    }
}

/// Count of `books` with `done` false.
fn pending(books: &[Book]) -> usize {
    books.iter().filter(|b| !b.done).count()
}

/// `Book::cover_path` decoded for the grid. `None` leaves a placeholder.
fn load_cover(book: &Book) -> Option<DynamicImage> {
    let path = book.cover_path.as_deref()?;
    let bytes = std::fs::read(path).ok()?;
    match grid::decode_resize(&bytes) {
        Ok(img) => Some(img),
        Err(e) => {
            log(format!("cover {}: {e}", path.display()));
            None
        }
    }
}

/// One page of cells, [`ui::header`] and [`ui::pager`].
#[allow(clippy::too_many_arguments)]
fn draw_page(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    books: &[Book],
    covers: &[Option<DynamicImage>],
    cfg: &Config,
    layout: grid::Layout,
    page: usize,
    total_pages: usize,
) {
    fb.fill_rect(0, 0, fb.var.xres, fb.var.yres, 0xFF);
    header::draw(fb, renderer, pending(books), books.len(), &cfg.out_dir);

    if books.is_empty() {
        draw_empty(fb, renderer, cfg);
    }

    let start = page * layout.page_size();
    let end = (start + layout.page_size()).min(books.len());
    for (slot, book) in books[start..end].iter().enumerate() {
        let (cx, cy) = layout.cell_xy(slot);
        let rect = grid::draw_book_cell(
            fb,
            renderer,
            cx,
            cy,
            layout.cell_h,
            covers.get(start + slot).and_then(|c| c.as_ref()),
            label_of(book),
        );
        if book.done {
            grid::draw_downloaded_badge(fb, rect);
        }
    }

    pager::draw(fb, renderer, page, total_pages, pending(books));
}

/// Message for an empty `books`, naming which of the three states holds.
fn draw_empty(fb: &mut Framebuffer, renderer: &mut TextRenderer, cfg: &Config) {
    let lines: Vec<String> = if !cfg.lists_anything() {
        vec![
            "Nothing is being listed".to_string(),
            "Open Settings and turn on a folder and a format".to_string(),
        ]
    } else if cfg.show_done {
        vec![
            "No DRM'd books found".to_string(),
            format!("Looked in {}", roots_summary(cfg)),
        ]
    } else {
        vec![
            "Nothing left to decrypt".to_string(),
            "Finished books are hidden — turn that off in Settings".to_string(),
        ]
    };

    let lh = renderer.line_height().max(1);
    for (i, line) in lines.iter().enumerate() {
        let w = renderer.measure_width(line);
        let x = ((fb.var.xres as i32 - w as i32) / 2).max(0);
        let y = (fb.var.yres / 3 + i as u32 * lh * 2) as i32;
        renderer.draw(fb, x, y, line, false);
    }
}

/// `Config::scan_roots` as one line.
fn roots_summary(cfg: &Config) -> String {
    let roots = cfg.scan_roots();
    let names: Vec<String> = roots.iter().map(|r| r.display().to_string()).collect();
    names.join("  and  ")
}

/// [`load_cover`] for each cell on `page`, refreshing that cell as its image
/// lands. Decoding a full page of JPEGs takes visible time on this CPU.
fn fill_covers(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    books: &[Book],
    covers: &mut [Option<DynamicImage>],
    layout: grid::Layout,
    page: usize,
) -> anyhow::Result<()> {
    let start = page * layout.page_size();
    let end = (start + layout.page_size()).min(books.len());
    for idx in start..end {
        if covers[idx].is_some() || books[idx].cover_path.is_none() {
            continue;
        }
        let Some(img) = load_cover(&books[idx]) else {
            continue;
        };
        covers[idx] = Some(img);

        let (cx, cy) = layout.cell_xy(idx - start);
        if cx < 0 || cy < 0 {
            continue;
        }
        let rect = grid::draw_book_cell(
            fb,
            renderer,
            cx,
            cy,
            layout.cell_h,
            covers[idx].as_ref(),
            label_of(&books[idx]),
        );
        if books[idx].done {
            grid::draw_downloaded_badge(fb, rect);
        }
        fb.send_update(cell_rect(cx, cy, layout.cell_h), WAVEFORM_MODE_DU)?;
    }
    Ok(())
}

/// One input event during a decrypt. `TouchEvent::Screenshot` captures the
/// live banner; `eink::screenshot::capture` restores the screen. Every other
/// event is dropped.
fn decrypt_input_event(fb: &mut Framebuffer, ev: InputEvent) {
    if ev == InputEvent::Touch(TouchEvent::Screenshot) {
        match eink::screenshot::capture(fb) {
            Ok(p) => log(format!("screenshot saved: {}", p.display())),
            Err(e) => log(format!("screenshot failed: {e:#}")),
        }
    }
}

/// Holds the panel for `linger`, servicing `input` throughout.
///
/// `Touch` holds an `EVIOCGRAB`, and `TouchEvent::Screenshot` queues on that
/// fd until `Input::next_deadline` reads it.
fn hold(fb: &mut Framebuffer, input: &mut Input, linger: Duration) -> anyhow::Result<()> {
    let until = Instant::now() + linger;
    while Instant::now() < until {
        match input.next_deadline(Some(until))? {
            InputEvent::Tick => {}
            ev => decrypt_input_event(fb, ev),
        }
    }
    Ok(())
}

/// Non-empty lines from `out`, on a thread. The receiver drains from a loop
/// that also polls `input`.
fn read_lines(out: std::process::ChildStdout) -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(out);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let msg = line.trim();
            if !msg.is_empty() && tx.send(msg.to_string()).is_err() {
                break;
            }
        }
    });
    rx
}

/// True once `engine::output_path(book.path, out_dir)` exists.
///
/// An engine build that ignores its third argument writes to
/// [`config::DEFAULT_OUT_DIR`]; one rename moves that across.
fn settle_output(book: &Book, out_dir: &Path) -> bool {
    let Some(expected) = engine::output_path(&book.path, out_dir) else {
        return false;
    };
    if expected.exists() {
        return true;
    }
    let Some(fallback) = engine::output_path(&book.path, Path::new(config::DEFAULT_OUT_DIR)) else {
        return false;
    };
    if fallback == expected || !fallback.exists() {
        return false;
    }
    log(format!(
        "engine ignored the out-folder argument; moving {} -> {}",
        fallback.display(),
        expected.display()
    ));
    if let Some(parent) = expected.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::rename(&fallback, &expected) {
        Ok(()) => true,
        Err(e) => {
            log(format!("move failed: {e}"));
            false
        }
    }
}

/// One `engine::decrypt` run, its stdout streamed into a banner.
///
/// Returns the banner message and whether [`settle_output`] found the file. A
/// zero exit with no file reports as a failure.
///
/// [`read_lines`] owns stdout; the loop waits on `input` at [`ENGINE_POLL`].
///
/// `Engine` prints nothing while a book decrypts, leaving `read_line` blocked
/// and the touch fd unpolled for that whole stretch.
fn decrypt_one(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    input: &mut Input,
    eng: &Engine,
    cfg: &Config,
    book: &Book,
) -> anyhow::Result<(String, bool)> {
    let short = short_title(&book.title, 34);
    // Two lines here and in the result banner: equal `toast::draw` footprints.
    let rect = toast::draw(fb, renderer, &format!("{short}\nStarting…"));
    fb.send_update(rect, WAVEFORM_MODE_GC16)?;

    let mut child = match eng
        .decrypt(&book.path, &cfg.out_dir)
        .stdout(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            log(format!("spawn failed: {e}"));
            return Ok((format!("{short}\nCould not start the engine"), false));
        }
    };

    let lines = child.stdout.take().map(read_lines);

    let mut last_draw = Instant::now();
    let mut latest: Option<String> = None;
    let status = loop {
        match input.next_deadline(Some(Instant::now() + ENGINE_POLL))? {
            InputEvent::Tick => {}
            ev => decrypt_input_event(fb, ev),
        }
        if let Some(rx) = lines.as_ref() {
            while let Ok(msg) = rx.try_recv() {
                log(format!("engine: {msg}"));
                latest = Some(msg);
            }
        }
        if last_draw.elapsed() >= TOAST_REDRAW_INTERVAL
            && let Some(msg) = latest.take()
        {
            let rect = toast::draw(fb, renderer, &format!("{short}\n{}", short_title(&msg, 40)));
            fb.send_update(rect, WAVEFORM_MODE_GC16)?;
            last_draw = Instant::now();
        }
        match child.try_wait() {
            Ok(Some(s)) => break Ok(s),
            Ok(None) => {}
            Err(e) => break Err(e),
        }
    };
    // Lines buffered between the last drain and the exit.
    if let Some(rx) = lines {
        while let Ok(msg) = rx.try_recv() {
            log(format!("engine: {msg}"));
        }
    }
    log(format!(
        "engine exit={status:?} for {}",
        book.path.display()
    ));
    if !matches!(status, Ok(ref s) if s.success()) {
        return Ok((format!("{short}\nFailed — see the log"), false));
    }

    if settle_output(book, &cfg.out_dir) {
        Ok((format!("{short}\nDecrypted"), true))
    } else {
        // `scan` drops books whose output exists, leaving the engine's own
        // skip path unreached here.
        Ok((
            format!("{short}\nEngine finished, but wrote no file"),
            false,
        ))
    }
}

/// One `engine::decrypt` run per `books` entry with `done` false.
///
/// `Touch` holds an `EVIOCGRAB`: an unpolled fd queues every gesture for the
/// length of a book. A failed run leaves the loop going.
fn decrypt_all(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    input: &mut Input,
    eng: &Engine,
    cfg: &Config,
    books: &[Book],
) -> anyhow::Result<String> {
    let todo: Vec<&Book> = books.iter().filter(|b| !b.done).collect();
    let total = todo.len();
    let started = Instant::now();
    let (mut done, mut failed) = (0usize, 0usize);

    let mut stopping = false;
    for (i, book) in todo.iter().enumerate() {
        // Drawn before the run: the bar carries `i`, the title carries `book`.
        let (rect, stop_rect) =
            toast::draw_progress_stop(fb, renderer, &short_title(&book.title, 30), i, total);
        fb.send_update(rect, WAVEFORM_MODE_GC16)?;

        // Inherited stdio, no pipe to drain. The wait blocks in
        // `next_deadline`: a gesture wakes it, an idle tick bounds the exit
        // check at [`ENGINE_POLL`].
        let status = match eng.decrypt(&book.path, &cfg.out_dir).spawn() {
            Ok(mut child) => loop {
                match child.try_wait() {
                    Ok(Some(s)) => break Ok(s),
                    Ok(None) => {}
                    Err(e) => break Err(e),
                }
                match input.next_deadline(Some(Instant::now() + ENGINE_POLL))? {
                    InputEvent::Tick => {}
                    // `child` runs to its own exit; the loop ends after it.
                    InputEvent::Touch(TouchEvent::Up { x, y })
                        if !stopping && toast::contains(stop_rect, x, y) =>
                    {
                        stopping = true;
                        log("batch: stop requested");
                        // `draw_progress` drops the button, marking the tap.
                        let rect = toast::draw_progress(
                            fb,
                            renderer,
                            "Stopping after this book…",
                            i,
                            total,
                        );
                        fb.send_update(rect, WAVEFORM_MODE_GC16)?;
                    }
                    ev => decrypt_input_event(fb, ev),
                }
            },
            Err(e) => Err(e),
        };

        let ok = matches!(status, Ok(ref s) if s.success()) && settle_output(book, &cfg.out_dir);
        log(format!(
            "batch {}/{}: {} exit={status:?} ok={ok}",
            i + 1,
            total,
            book.path.display()
        ));
        if ok {
            done += 1;
        } else {
            failed += 1;
        }
        if stopping {
            break;
        }
    }

    let ran = done + failed;
    let left = total - ran;
    // Bar at `ran` ahead of the summary banner.
    let rect = toast::draw_progress(fb, renderer, "Done", ran, total);
    fb.send_update(rect, WAVEFORM_MODE_GC16)?;
    log(format!(
        "batch finished in {:?}: {done} decrypted, {failed} failed, {left} left",
        started.elapsed()
    ));

    Ok(batch_summary(done, failed, left))
}

/// The banner `decrypt_all` ends on. `left` counts books a stop skipped.
fn batch_summary(done: usize, failed: usize, left: usize) -> String {
    let head = match (done, failed) {
        (0, 0) => "Nothing to do".to_string(),
        (d, 0) => format!("Decrypted {d}"),
        (0, f) => format!("All {f} failed"),
        (d, f) => format!("Decrypted {d}, {f} failed"),
    };
    match (left, failed) {
        (0, 0) => head,
        (0, _) => format!("{head}\nSee the log"),
        (n, 0) => format!("{head}\nStopped, {n} left"),
        (n, _) => format!("{head}\nStopped, {n} left — see the log"),
    }
}

pub fn run() -> anyhow::Result<()> {
    log(format!(
        "kfxdedrm-fe {} starting",
        env!("CARGO_PKG_VERSION")
    ));

    // ---- Device setup ------------------------------------------------------
    // `ui::setup` draws and takes taps: `Framebuffer` and `Input` precede
    // `engine::locate`, and `Buttons::open` grabs the bezel keys.
    let mut renderer = TextRenderer::load(FONT_PX)?;
    log(format!("fonts: {}", renderer.chain_description()));

    let mut orient = Orientation::detect();
    let mut fb = Framebuffer::open()?;
    let touch = Touch::open(orient, fb.var.xres, fb.var.yres)?;
    let buttons = match Buttons::open() {
        Ok(Some(b)) => {
            log("buttons: grabbed gpio-keys");
            Some(b)
        }
        Ok(None) => {
            log("buttons: none — touch-only");
            None
        }
        Err(e) => {
            log(format!("buttons: {e:#} — touch-only"));
            None
        }
    };
    let mut input = Input::new(touch, buttons);
    input.set_orientation(orient);

    // ---- The engine ---------------------------------------------------------
    let eng = match engine::locate() {
        Ok(e) => {
            log(format!("engine: {}", e.exe().display()));
            e
        }
        Err(reason) => {
            log(format!("engine: {reason:?}"));
            return setup::run(&mut fb, &mut input, &mut renderer, reason);
        }
    };

    // ---- Settings + first scan ---------------------------------------------
    let cfg_path = config_path();
    let mut cfg = Config::load(&cfg_path);
    log(format!(
        "settings: roots={:?} kfx={} mobi={} out={} show_done={}",
        cfg.scan_roots(),
        cfg.types_kfx,
        cfg.types_mobi,
        cfg.out_dir.display(),
        cfg.show_done
    ));

    let mut books = scan::scan(&cfg);
    let mut covers: Vec<Option<DynamicImage>> = vec![None; books.len()];
    log(format!(
        "scan: {} books, {} to decrypt",
        books.len(),
        pending(&books)
    ));

    // ---- Grid loop ----------------------------------------------------------
    let mut layout =
        grid::Layout::compute(fb.var.xres, fb.var.yres, header::MARGIN, pager::STRIP_H);
    let mut page = 0usize;
    // `repaint!` computes this from `books` and `layout` before any read.
    let mut total_pages;

    macro_rules! repaint {
        () => {{
            total_pages = pager::n_pages(books.len(), layout.page_size());
            page = page.min(total_pages.saturating_sub(1));
            draw_page(
                &mut fb,
                &mut renderer,
                &books,
                &covers,
                &cfg,
                layout,
                page,
                total_pages,
            );
            fb.send_update(full_rect(&fb), WAVEFORM_MODE_GC16)?;
            fill_covers(&mut fb, &mut renderer, &books, &mut covers, layout, page)?;
        }};
    }

    // Rebuilds `books` and `covers` together: an index into the old `books`
    // does not carry over.
    macro_rules! rescan {
        () => {{
            books = scan::scan(&cfg);
            covers = vec![None; books.len()];
            log(format!(
                "rescan: {} books, {} to decrypt",
                books.len(),
                pending(&books)
            ));
        }};
    }

    // One cell in place, dropping a press outline without a panel flash.
    macro_rules! redraw_cell {
        ($slot:expr) => {{
            let slot = $slot;
            let idx = page * layout.page_size() + slot;
            let (cx, cy) = layout.cell_xy(slot);
            if idx < books.len() && cx >= 0 && cy >= 0 {
                let rect = grid::draw_book_cell(
                    &mut fb,
                    &mut renderer,
                    cx,
                    cy,
                    layout.cell_h,
                    covers[idx].as_ref(),
                    label_of(&books[idx]),
                );
                if books[idx].done {
                    grid::draw_downloaded_badge(&mut fb, rect);
                }
                fb.send_update(cell_rect(cx, cy, layout.cell_h), WAVEFORM_MODE_DU)?;
            }
        }};
    }

    repaint!();

    let mut armed: Option<Armed> = None;

    loop {
        // A held finger emits near-continuous jitter, keeping the touch fd
        // readable. `next_deadline` reaches an absolute `Instant` through
        // that; an idle timeout does not.
        let deadline = armed.as_ref().map(|a| a.down_at + ARM_THRESHOLD);
        match input.next_deadline(deadline)? {
            InputEvent::Touch(TouchEvent::Down { x, y }) => {
                // `grid::outline_cell` marks the press. [`ARM_THRESHOLD`]
                // decides the rest.
                if pager::hit(x, y, fb.var.xres, fb.var.yres, total_pages, pending(&books))
                    .is_none()
                    && let Some(slot) = layout.cell_at_tap(x, y, books.len())
                {
                    let idx = page * layout.page_size() + slot;
                    let (cx, cy) = layout.cell_xy(slot);
                    if idx < books.len() && cx >= 0 && cy >= 0 {
                        grid::outline_cell(&mut fb, cx, cy, layout.cell_h);
                        fb.send_update(cell_rect(cx, cy, layout.cell_h), WAVEFORM_MODE_DU)?;
                        armed = Some(Armed {
                            slot,
                            idx,
                            down_at: Instant::now(),
                            at: (x, y),
                        });
                    }
                }
            }

            InputEvent::Touch(TouchEvent::Up { x, y }) => {
                // `Tick` takes `armed` at [`ARM_THRESHOLD`]. `armed` set here
                // marks a release before it.
                if let Some(a) = armed.take() {
                    if x.abs_diff(a.at.0) > ARM_SLOP_PX || y.abs_diff(a.at.1) > ARM_SLOP_PX {
                        redraw_cell!(a.slot);
                    } else {
                        log(format!("short tap ({:?})", a.down_at.elapsed()));
                        let rect = toast::draw(&mut fb, &mut renderer, "Hold to decrypt");
                        fb.send_update(rect, WAVEFORM_MODE_GC16)?;
                        hold(&mut fb, &mut input, HINT_LINGER)?;
                        // `repaint!` covers the banner and the outline.
                        repaint!();
                    }
                    continue;
                }
                match pager::hit(x, y, fb.var.xres, fb.var.yres, total_pages, pending(&books)) {
                    Some(pager::PagerHit::Exit) => return Ok(()),
                    Some(pager::PagerHit::Settings) => {
                        let before = cfg.clone();
                        configmenu::run(&mut fb, &mut input, &mut renderer, &mut cfg, &mut orient)?;
                        if cfg != before {
                            if let Err(e) = cfg.store(&cfg_path) {
                                log(format!("settings save failed: {e}"));
                            }
                            rescan!();
                            page = 0;
                        }
                        repaint!();
                    }
                    Some(pager::PagerHit::DecryptAll) => {
                        // `pager::hit` returns `DecryptAll` only above zero `pending`.
                        let msg =
                            decrypt_all(&mut fb, &mut renderer, &mut input, &eng, &cfg, &books)?;
                        let rect = toast::draw_download_done(&mut fb, &mut renderer, &msg);
                        fb.send_update(rect, WAVEFORM_MODE_GC16)?;
                        hold(&mut fb, &mut input, RESULT_LINGER)?;
                        rescan!();
                        page = 0;
                        repaint!();
                    }
                    Some(pager::PagerHit::Prev) if page > 0 => {
                        page -= 1;
                        repaint!();
                    }
                    Some(pager::PagerHit::Next) if page + 1 < total_pages => {
                        page += 1;
                        repaint!();
                    }
                    // A dead edge, or the margins around the grid.
                    _ => {}
                }
            }

            InputEvent::Touch(TouchEvent::Screenshot) => {
                let _ = eink::screenshot::capture(&mut fb);
            }

            InputEvent::Page(dir) => match dir {
                PageButton::Next if page + 1 < total_pages => {
                    page += 1;
                    repaint!();
                }
                PageButton::Prev if page > 0 => {
                    page -= 1;
                    repaint!();
                }
                _ => {}
            },

            InputEvent::Tick => {
                // The [`ARM_THRESHOLD`] deadline, or an idle poll carrying only
                // an `Orientation` change.
                let fire = armed
                    .as_ref()
                    .is_some_and(|a| a.down_at.elapsed() >= ARM_THRESHOLD);
                if let Some(a) = armed.take_if(|_| fire) {
                    // Drift past [`ARM_SLOP_PX`] cancels. One cell repaints; a
                    // panel flash on every mis-swipe costs more than the
                    // outline.
                    let (px, py) = input.touch_pos();
                    if px.abs_diff(a.at.0) > ARM_SLOP_PX || py.abs_diff(a.at.1) > ARM_SLOP_PX {
                        redraw_cell!(a.slot);
                        continue;
                    }
                    let Some(book) = books.get(a.idx).cloned() else {
                        repaint!();
                        continue;
                    };
                    if book.done {
                        // `engine::decrypt` skips a book whose output exists.
                        let rect = toast::draw_download_done(
                            &mut fb,
                            &mut renderer,
                            &format!("{}\nAlready decrypted", short_title(&book.title, 34)),
                        );
                        fb.send_update(rect, WAVEFORM_MODE_GC16)?;
                        hold(&mut fb, &mut input, RESULT_LINGER)?;
                        repaint!();
                        continue;
                    }

                    // [`ARM_DWELL`] holds the cue on the panel ahead of the
                    // banner.
                    let (cx, cy) = layout.cell_xy(a.slot);
                    if cx >= 0 && cy >= 0 {
                        grid::draw_arm_cue(&mut fb, cx, cy, layout.cell_h);
                        fb.send_update(cell_rect(cx, cy, layout.cell_h), WAVEFORM_MODE_DU)?;
                        hold(&mut fb, &mut input, ARM_DWELL)?;
                    }
                    // `armed` is taken, leaving the eventual lift inert.
                    log(format!("decrypting {}", book.path.display()));
                    let (msg, ok) =
                        decrypt_one(&mut fb, &mut renderer, &mut input, &eng, &cfg, &book)?;
                    log(format!("result: ok={ok} {}", msg.replace('\n', " — ")));
                    let rect = toast::draw_download_done(&mut fb, &mut renderer, &msg);
                    fb.send_update(rect, WAVEFORM_MODE_GC16)?;
                    hold(&mut fb, &mut input, RESULT_LINGER)?;

                    rescan!();
                    repaint!();
                    continue;
                }

                let o = Orientation::detect();
                if o != orient {
                    orient = o;
                    input.set_orientation(o);
                    layout = grid::Layout::compute(
                        fb.var.xres,
                        fb.var.yres,
                        header::MARGIN,
                        pager::STRIP_H,
                    );
                    repaint!();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batch_that_ran_to_the_end_reports_no_remainder() {
        assert_eq!(batch_summary(0, 0, 0), "Nothing to do");
        assert_eq!(batch_summary(4, 0, 0), "Decrypted 4");
        assert_eq!(batch_summary(0, 2, 0), "All 2 failed\nSee the log");
        assert_eq!(batch_summary(3, 1, 0), "Decrypted 3, 1 failed\nSee the log");
    }

    #[test]
    fn a_stopped_batch_names_the_books_it_left() {
        assert_eq!(batch_summary(2, 0, 5), "Decrypted 2\nStopped, 5 left");
        assert_eq!(
            batch_summary(2, 1, 5),
            "Decrypted 2, 1 failed\nStopped, 5 left — see the log"
        );
    }

    /// [`hold`] is the only wait in this module. `input` goes unpolled across
    /// any other.
    #[test]
    fn no_wait_here_blocks_the_touch_fd() {
        // `blocking` is split, leaving `include_str!` no literal to match.
        let blocking = concat!("std::thread", "::sleep(");
        let src = include_str!("app.rs");
        assert_eq!(src.matches(blocking).count(), 0, "{blocking} bypasses hold");
        assert!(src.contains("fn hold("));
    }

    #[test]
    fn a_title_longer_than_the_banner_is_ellipsized() {
        assert_eq!(short_title("Short", 10), "Short");
        assert_eq!(short_title("0123456789", 10), "0123456789");
        assert_eq!(short_title("01234567890", 10), "012345678…");
        // Char counts, not bytes: a multi-byte title keeps its boundaries.
        assert_eq!(short_title("日本語の題名です", 4), "日本語…");
    }
}
