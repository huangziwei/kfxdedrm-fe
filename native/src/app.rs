//! [`run`]: a paginated grid of `[crate::scan::Book]`, with
//! [`crate::ui::configmenu`] as a blocking overlay. A held cover runs
//! [`decrypt_one`]; the toolbar runs [`decrypt_all`].

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use image::DynamicImage;

use crate::config::{self, Config};
use crate::convert::{self, Converter, Targets};
use crate::eink;
use crate::eink::buttons::{Buttons, PageButton};
use crate::eink::fb::{Framebuffer, MxcfbRect, WAVEFORM_MODE_DU, WAVEFORM_MODE_GC16};
use crate::eink::input::{Input, InputEvent};
use crate::eink::touch::{Touch, TouchEvent};
use crate::engine::{self, Engine};
use crate::font;
use crate::install;
use crate::log;
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
/// Result banner time after [`install_addons`], which reports one line per
/// add-on and is worth reading before the panel takes it away.
const INSTALL_LINGER: Duration = Duration::from_millis(2600);

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
    header::draw(fb, renderer, pending(books), books.len());

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
            "Open Settings and pick a folder and a format".to_string(),
        ]
    } else if cfg.show_done {
        vec![
            "No DRM'd books found".to_string(),
            format!("Looked in {}", folders_summary(cfg)),
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

/// `Config::scan_dirs` as one line: the folder at one, a count above that.
fn folders_summary(cfg: &Config) -> String {
    match cfg.scan_dirs.as_slice() {
        [] => "no folder".to_string(),
        [one] => one.display().to_string(),
        many => format!("{} folders", many.len()),
    }
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

/// True once `engine::output_path` of `book` exists under `config::OUT_DIR`.
///
/// The engine writes there and nowhere else, whatever out folder it is handed
/// — see `engine`'s module header — so this is a look, not a move.
fn decrypted(book: &Book) -> bool {
    engine::output_path(&book.path, Path::new(config::OUT_DIR)).is_some_and(|p| p.exists())
}

/// How a [`convert_outputs`] step announces itself. The two callers want
/// different banners and only one of them has a button to keep alive.
enum StepBanner<'a> {
    /// [`decrypt_one`]: the book's title over the step line.
    One(&'a str),
    /// [`decrypt_all`]: the batch bar, its Stop button live across the step.
    ///
    /// The banner and the button are redrawn per step.
    Batch {
        /// Position in the batch, for the bar.
        done: usize,
        total: usize,
        /// Set by a tap on the button. `decrypt_all` reads it after the book.
        stopping: &'a mut bool,
    },
}

/// `targets`'s steps over `decrypted`, each run to its own exit.
///
/// Returns the `convert::Kind`s that failed, empty being the good case. A step
/// whose `output` is present is skipped.
fn convert_outputs(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    input: &mut Input,
    conv: &Converter,
    targets: Targets,
    decrypted: &Path,
    banner: &mut StepBanner,
) -> anyhow::Result<Vec<convert::Kind>> {
    let mut failed = Vec::new();
    for step in targets.steps(decrypted) {
        if step.output.exists() {
            continue;
        }
        // The EPUB step reads the KFX step's output; a failure upstream leaves
        // nothing to open.
        if !step.input.exists() {
            log(format!(
                "convert {}: no input at {}",
                step.kind.label(),
                step.input.display()
            ));
            failed.push(step.kind);
            continue;
        }

        // Cleared by the tap that takes it: `draw_progress` below drops the
        // button, and a second tap on the same spot must not re-announce.
        let mut stop_rect = draw_step_banner(fb, renderer, banner, step.kind)?;

        let status = match conv.convert(&step).spawn() {
            Ok(mut child) => loop {
                match child.try_wait() {
                    Ok(Some(s)) => break Ok(s),
                    Ok(None) => {}
                    Err(e) => break Err(e),
                }
                match input.next_deadline(Some(Instant::now() + ENGINE_POLL))? {
                    InputEvent::Tick => {}
                    // Same contract as the engine wait: the child runs to its
                    // own exit and the batch ends after this book.
                    InputEvent::Touch(TouchEvent::Up { x, y })
                        if stop_rect.is_some_and(|r| toast::contains(r, x, y)) =>
                    {
                        if let StepBanner::Batch {
                            done,
                            total,
                            stopping,
                        } = banner
                        {
                            **stopping = true;
                            log("batch: stop requested");
                            // Dropping the button marks the tap.
                            let rect = toast::draw_progress(
                                fb,
                                renderer,
                                "Stopping after this book…",
                                *done,
                                *total,
                            );
                            fb.send_update(rect, WAVEFORM_MODE_GC16)?;
                        }
                        stop_rect = None;
                    }
                    ev => decrypt_input_event(fb, ev),
                }
            },
            Err(e) => Err(e),
        };

        let ok = matches!(status, Ok(ref s) if s.success()) && step.output.exists();
        log(format!(
            "convert {} {} -> {} exit={status:?} ok={ok}",
            step.kind.label(),
            step.input.display(),
            step.output.display()
        ));
        if !ok {
            // A half-written `step.output` reads as a finished one.
            if let Err(e) = remove_if_present(&step.output) {
                log(format!("cleanup {}: {e}", step.output.display()));
            }
            failed.push(step.kind);
        }
    }
    Ok(failed)
}

/// The banner one [`convert_outputs`] step runs under, and the Stop button's
/// hit rect while there is one.
///
/// [`StepBanner::Batch`] gives its one title line to the step. Both footprints
/// match `toast::draw_download_done`.
fn draw_step_banner(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    banner: &StepBanner,
    kind: convert::Kind,
) -> anyhow::Result<Option<MxcfbRect>> {
    let (rect, stop_rect) = match banner {
        StepBanner::One(title) => (
            toast::draw_download_done(fb, renderer, &format!("{title}\n{}", kind.progress())),
            None,
        ),
        // A batch asked to stop keeps the bar and loses the button.
        StepBanner::Batch {
            done,
            total,
            stopping,
        } if **stopping => (
            toast::draw_progress(fb, renderer, kind.progress(), *done, *total),
            None,
        ),
        StepBanner::Batch { done, total, .. } => {
            let (rect, stop) =
                toast::draw_progress_stop(fb, renderer, kind.progress(), *done, *total);
            (rect, Some(stop))
        }
    };
    fb.send_update(rect, WAVEFORM_MODE_GC16)?;
    Ok(stop_rect)
}

/// `remove_file`, with an already-absent path reading as success.
fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// The second banner line after a decrypt that produced its file.
fn result_line(failed: &[convert::Kind]) -> String {
    match failed {
        [] => "Decrypted".to_string(),
        [one] => format!("Decrypted, no {} — see the log", one.label()),
        many => format!("Decrypted, {} conversions failed", many.len()),
    }
}

/// One `engine::decrypt` run, its stdout streamed into a banner, then
/// [`convert_outputs`] over what it wrote.
///
/// Returns the banner message and whether every output the settings ask for
/// is present. A zero exit with no file reports as a failure.
///
/// [`read_lines`] owns stdout; the loop waits on `input` at [`ENGINE_POLL`].
///
/// `Engine` prints nothing while a book decrypts, leaving `read_line` blocked
/// and the touch fd unpolled for that whole stretch.
#[allow(clippy::too_many_arguments)]
fn decrypt_one(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    input: &mut Input,
    eng: &Engine,
    conv: Option<&Converter>,
    targets: Targets,
    book: &Book,
) -> anyhow::Result<(String, bool)> {
    let short = short_title(&book.title, 34);

    // `scan::Book::done` counts the conversions; `decrypted` counts the
    // engine's own output.
    if !decrypted(book) {
        if let Some(msg) = run_engine(fb, renderer, input, eng, book, &short)? {
            return Ok((msg, false));
        }
        if !decrypted(book) {
            return Ok((
                format!("{short}\nEngine finished, but wrote no file"),
                false,
            ));
        }
    } else {
        log(format!("already decrypted: {}", book.path.display()));
    }

    let failed = match (
        conv,
        engine::output_path(&book.path, Path::new(config::OUT_DIR)),
    ) {
        (Some(c), Some(out)) => {
            let mut banner = StepBanner::One(&short);
            convert_outputs(fb, renderer, input, c, targets, &out, &mut banner)?
        }
        _ => Vec::new(),
    };
    let ok = failed.is_empty();
    Ok((format!("{short}\n{}", result_line(&failed)), ok))
}

/// The `engine::decrypt` half of [`decrypt_one`], stdout streamed into a
/// banner. `Some` carries the banner message for a run that failed.
fn run_engine(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    input: &mut Input,
    eng: &Engine,
    book: &Book,
    short: &str,
) -> anyhow::Result<Option<String>> {
    // Two lines here and in the result banner: equal `toast::draw` footprints.
    let rect = toast::draw(fb, renderer, &format!("{short}\nStarting…"));
    fb.send_update(rect, WAVEFORM_MODE_GC16)?;

    let mut child = match eng
        .decrypt(&book.path, Path::new(config::OUT_DIR))
        .stdout(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            log(format!("spawn failed: {e}"));
            return Ok(Some(format!("{short}\nCould not start the engine")));
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
        return Ok(Some(format!("{short}\nFailed — see the log")));
    }
    Ok(None)
}

/// One `engine::decrypt` run per `books` entry with `done` false, each
/// followed by [`convert_outputs`].
///
/// `Touch` holds an `EVIOCGRAB`: an unpolled fd queues every gesture for the
/// length of a book. A failed run leaves the loop going, and a book whose
/// conversions failed counts against the summary the way a failed decrypt
/// does — `scan::Book::done` reads both the same way.
#[allow(clippy::too_many_arguments)]
fn decrypt_all(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    input: &mut Input,
    eng: &Engine,
    conv: Option<&Converter>,
    targets: Targets,
    books: &[Book],
) -> anyhow::Result<String> {
    let todo: Vec<&Book> = books.iter().filter(|b| !b.done).collect();
    let total = todo.len();
    let started = Instant::now();
    let (mut done, mut failed) = (0usize, 0usize);

    let mut stopping = false;
    for (i, book) in todo.iter().enumerate() {
        let short = short_title(&book.title, 30);
        // Drawn before the book's work: the bar carries `i`, the title
        // carries `book`.
        let (rect, stop_rect) = toast::draw_progress_stop(fb, renderer, &short, i, total);
        fb.send_update(rect, WAVEFORM_MODE_GC16)?;

        // `decrypted` alone, with the conversions left — see [`decrypt_one`].
        let mut ok = decrypted(book);
        if ok {
            log(format!(
                "batch {}/{}: already decrypted {}",
                i + 1,
                total,
                book.path.display()
            ));
        } else {
            let status = match eng.decrypt(&book.path, Path::new(config::OUT_DIR)).spawn() {
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

            ok = matches!(status, Ok(ref s) if s.success()) && decrypted(book);
            log(format!(
                "batch {}/{}: {} exit={status:?} ok={ok}",
                i + 1,
                total,
                book.path.display()
            ));
        }

        if ok
            && let Some(c) = conv
            && let Some(out) = engine::output_path(&book.path, Path::new(config::OUT_DIR))
        {
            let mut banner = StepBanner::Batch {
                done: i,
                total,
                stopping: &mut stopping,
            };
            let failed = convert_outputs(fb, renderer, input, c, targets, &out, &mut banner)?;
            ok = failed.is_empty();
        }

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

/// Fetch or update both add-ons, drawing progress, and report what landed.
///
/// The work runs on a worker thread. Neither the release list nor the download
/// is something the panel can poll, and a screen that stops taking taps for a
/// minute reads as a crash — so the transfer blocks a thread while this loop
/// keeps `input` drained, repaints the banner as steps arrive, and sets the
/// flag the download reads between chunks when Cancel is tapped.
///
/// Returns the summary to show, one line per add-on.
fn install_addons(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    input: &mut Input,
) -> anyhow::Result<String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel::<install::Step>();
    let flag = Arc::clone(&cancel);
    let worker = std::thread::spawn(move || {
        install::install_all(
            &install::record::path(),
            &|step| {
                let _ = tx.send(step);
            },
            &flag,
        )
    });

    let mut title = "Add-ons".to_string();
    let mut detail = "Asking GitHub…".to_string();
    let (rect, cancel_rect) = toast::draw_download(fb, renderer, &title, &detail);
    fb.send_update(rect, WAVEFORM_MODE_GC16)?;
    let mut painted = Instant::now();
    let mut stale = false;

    loop {
        while let Ok(step) = rx.try_recv() {
            title = step.title();
            detail = step.detail;
            stale = true;
        }
        // Every repaint is a full-banner GC16, and a percentage moves several
        // times a second. The floor is what keeps that from flashing the panel.
        if stale && painted.elapsed() >= TOAST_REDRAW_INTERVAL {
            let (rect, _) = toast::draw_download(fb, renderer, &title, &detail);
            fb.send_update(rect, WAVEFORM_MODE_GC16)?;
            painted = Instant::now();
            stale = false;
        }
        // Every step is drained above before this is read, so nothing the
        // worker sent is lost by leaving here.
        if worker.is_finished() {
            break;
        }

        match input.next_deadline(Some(Instant::now() + ENGINE_POLL))? {
            InputEvent::Touch(TouchEvent::Up { x, y })
                if !cancel.load(Ordering::Relaxed) && toast::contains(cancel_rect, x, y) =>
            {
                // The add-on in flight stops at its next chunk.
                cancel.store(true, Ordering::Relaxed);
                log("add-ons: cancelled");
                let rect = toast::draw_download_done(fb, renderer, &format!("{title}\nStopping…"));
                fb.send_update(rect, WAVEFORM_MODE_GC16)?;
                painted = Instant::now();
                stale = false;
            }
            ev => decrypt_input_event(fb, ev),
        }
    }

    let lines = worker
        .join()
        .unwrap_or_else(|_| vec!["Add-ons: failed".to_string()]);
    for line in &lines {
        log(line);
    }
    Ok(lines.join("\n"))
}

/// [`install_addons`], its summary banner, and the pause to read it.
fn fetch_addons(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    input: &mut Input,
) -> anyhow::Result<()> {
    let msg = install_addons(fb, renderer, input)?;
    let rect = toast::draw_download_done(fb, renderer, &msg);
    fb.send_update(rect, WAVEFORM_MODE_GC16)?;
    hold(fb, input, INSTALL_LINGER)
}

/// The banner a decrypt gets when there is no engine to run it.
fn no_engine_banner(
    fb: &mut Framebuffer,
    renderer: &mut TextRenderer,
    input: &mut Input,
) -> anyhow::Result<()> {
    let rect = toast::draw_download_done(
        fb,
        renderer,
        "kfxdedrm is not installed\nOpen Settings and tap Add-ons",
    );
    fb.send_update(rect, WAVEFORM_MODE_GC16)?;
    hold(fb, input, RESULT_LINGER)
}

/// What the two probes found, into the log.
fn log_addons(engine: Result<&Engine, engine::Missing>, converter: Option<&Converter>) {
    match engine {
        Ok(e) => log(format!("engine: {}", e.exe().display())),
        Err(reason) => log(format!("engine: {reason:?}")),
    }
    match converter {
        Some(c) => log(format!("converter: {}", c.exe().display())),
        None => log(format!("converter: none in {}", convert::BIN_DIR)),
    }
}

/// What `ui::configmenu` says about the two add-ons: whether each one runs
/// here, and which release this app fetched for it.
///
/// The probes decide what is installed and `record` only names it. A record
/// left behind by a copy that has since been deleted is not an install, and a
/// copy installed by hand is one the record has never heard of.
fn addons_state(
    record: &install::record::Record,
    engine: Option<&Engine>,
    converter: Option<&Converter>,
) -> configmenu::AddOns {
    let seen = |present: bool, key: &str| configmenu::AddOn {
        present,
        tag: present
            .then(|| record.get(key).map(str::to_string))
            .flatten(),
    };
    configmenu::AddOns {
        engine: seen(engine.is_some(), install::SOURCES[0].key),
        bokai: seen(converter.is_some(), install::SOURCES[1].key),
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

    // ---- The engine and the converter --------------------------------------
    // A missing `located` reaches `no_engine_banner`. A missing `converter`
    // reads `convert::Targets` as both switches off.
    let mut located = engine::locate();
    let mut converter = convert::locate();
    log_addons(located.as_ref().map_err(|e| *e), converter.as_ref());

    // No engine opens on the offer to fetch one. Either answer opens the app
    // afterwards; this is where it is easiest to say yes, not a gate.
    if let Some(reason) = located.as_ref().err().copied()
        && setup::run(&mut fb, &mut input, &mut renderer, reason)? == setup::Choice::Install
    {
        fetch_addons(&mut fb, &mut renderer, &mut input)?;
        located = engine::locate();
        converter = convert::locate();
        log_addons(located.as_ref().map_err(|e| *e), converter.as_ref());
    }
    let mut eng = located.ok();

    // ---- Settings + first scan ---------------------------------------------
    let cfg_path = config_path();
    let mut cfg = Config::load(&cfg_path);
    let mut targets = Targets::new(&cfg, converter.as_ref());
    log(format!(
        "settings: folders={:?} kfx={} mobi={} show_done={} targets={targets:?}",
        cfg.scan_dirs, cfg.types_kfx, cfg.types_mobi, cfg.show_done
    ));

    let mut books = scan::scan(&cfg, targets);
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
            books = scan::scan(&cfg, targets);
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
                        // A book downloaded during this run puts a new folder
                        // on the page.
                        let folders = scan::candidates(&cfg);
                        log(format!("folders: {folders:?}"));
                        // The panel comes back for one thing other than
                        // Done, and that one needs the panel it was drawn on:
                        // fetch, then reopen it showing what landed.
                        let mut fetched = false;
                        loop {
                            let record = install::record::Record::load(&install::record::path());
                            let addons = addons_state(&record, eng.as_ref(), converter.as_ref());
                            let exit = configmenu::run(
                                &mut fb,
                                &mut input,
                                &mut renderer,
                                &mut cfg,
                                &folders,
                                &mut orient,
                                &addons,
                            )?;
                            if exit == configmenu::Exit::Done {
                                break;
                            }
                            fetch_addons(&mut fb, &mut renderer, &mut input)?;
                            let relocated = engine::locate();
                            converter = convert::locate();
                            log_addons(relocated.as_ref().map_err(|e| *e), converter.as_ref());
                            eng = relocated.ok();
                            fetched = true;
                        }
                        if cfg != before
                            && let Err(e) = cfg.store(&cfg_path)
                        {
                            log(format!("settings save failed: {e}"));
                        }
                        // `Book::done` is read against these, and an install
                        // moves them as surely as a tap on a chip does.
                        if cfg != before || fetched {
                            targets = Targets::new(&cfg, converter.as_ref());
                            rescan!();
                            page = 0;
                        }
                        repaint!();
                    }
                    Some(pager::PagerHit::DecryptAll) => {
                        let Some(engine) = eng.as_ref() else {
                            no_engine_banner(&mut fb, &mut renderer, &mut input)?;
                            repaint!();
                            continue;
                        };
                        // `pager::hit` returns `DecryptAll` only above zero `pending`.
                        let msg = decrypt_all(
                            &mut fb,
                            &mut renderer,
                            &mut input,
                            engine,
                            converter.as_ref(),
                            targets,
                            &books,
                        )?;
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

                    // Ahead of the cue that animates the hold.
                    let Some(engine) = eng.as_ref() else {
                        no_engine_banner(&mut fb, &mut renderer, &mut input)?;
                        repaint!();
                        continue;
                    };

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
                    let (msg, ok) = decrypt_one(
                        &mut fb,
                        &mut renderer,
                        &mut input,
                        engine,
                        converter.as_ref(),
                        targets,
                        &book,
                    )?;
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
    fn the_result_line_names_a_conversion_that_did_not_land() {
        assert_eq!(result_line(&[]), "Decrypted");
        // One failure is worth naming; past that the log is the only place
        // the banner could fit them.
        assert_eq!(
            result_line(&[convert::Kind::Epub]),
            "Decrypted, no EPUB — see the log"
        );
        assert_eq!(
            result_line(&[convert::Kind::Kfx, convert::Kind::Epub]),
            "Decrypted, 2 conversions failed"
        );
    }

    #[test]
    fn the_panel_is_told_what_runs_here_and_the_record_only_names_it() {
        let mut record = install::record::Record::default();
        record.set("engine", "v10.0.30");
        record.set("bokai", "v0.1.3");

        // A `record` entry is not an install; the probes decide.
        let none = addons_state(&record, None, None);
        assert!(!none.engine.present && none.engine.tag.is_none());
        assert!(!none.bokai.present && none.bokai.tag.is_none());

        // The keys the panel reads are the ones `install` writes, not a second
        // spelling of them.
        assert_eq!(install::SOURCES[0].key, "engine");
        assert_eq!(install::SOURCES[1].key, "bokai");
        assert_eq!(record.get(install::SOURCES[0].key), Some("v10.0.30"));
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
