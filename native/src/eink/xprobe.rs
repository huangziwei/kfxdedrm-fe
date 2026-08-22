//! `--probe-x`: dump the X server's extensions, request limits, paint timings
//! and `/dev/fb0` geometry to [`DUMP_PATH`], in a form two devices diff against
//! each other.

use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use x11rb::connection::{Connection, RequestConnection as _};
use x11rb::protocol::xproto::{
    AtomEnum, BackingStore, ConnectionExt, CreateGCAux, CreateWindowAux, EventMask, ImageFormat,
    PropMode, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

/// Where the dump lands, on the user partition.
const OUT_PATH: &str = "/mnt/us/kfxdedrm-fe-xprobe.txt";

pub fn run() -> Result<()> {
    let mut o = String::new();
    let _ = writeln!(o, "== kfxdedrm-fe X/eink probe ==");
    let _ = writeln!(o, "picker {}", env!("CARGO_PKG_VERSION"));

    probe_x(&mut o);
    probe_backing_store(&mut o);
    probe_fb_readback(&mut o);
    probe_eink_paths(&mut o);

    println!("{o}");
    if let Err(e) = std::fs::write(OUT_PATH, &o) {
        eprintln!("xprobe: could not write {OUT_PATH}: {e}");
    } else {
        println!("xprobe: wrote {OUT_PATH}");
    }
    Ok(())
}

fn probe_x(o: &mut String) {
    let (conn, screen_num) = match x11rb::connect(None) {
        Ok(c) => c,
        Err(e) => {
            let _ = writeln!(o, "\n[X] connect FAILED: {e}");
            return;
        }
    };
    let setup = conn.setup().clone();
    let screen = setup.roots[screen_num].clone();

    let _ = writeln!(o, "\n[X server]");
    let _ = writeln!(
        o,
        "vendor={:?} release={} proto={}.{}",
        String::from_utf8_lossy(&setup.vendor),
        setup.release_number,
        setup.protocol_major_version,
        setup.protocol_minor_version
    );
    let _ = writeln!(
        o,
        "maximum_request_length={} units ({} bytes)",
        setup.maximum_request_length,
        setup.maximum_request_length as usize * 4
    );
    // Equal to the line above when BIG-REQUESTS is absent.
    let _ = writeln!(
        o,
        "maximum_request_bytes()={} (BIG-REQUESTS {})",
        conn.maximum_request_bytes(),
        if conn.maximum_request_bytes() > setup.maximum_request_length as usize * 4 {
            "active"
        } else {
            "ABSENT"
        }
    );
    let _ = writeln!(
        o,
        "image_byte_order={:?} bitmap_scanline_unit={} pad={}",
        setup.image_byte_order, setup.bitmap_format_scanline_unit, setup.bitmap_format_scanline_pad
    );
    for f in &setup.pixmap_formats {
        let _ = writeln!(
            o,
            "pixmap_format depth={} bpp={} pad={}",
            f.depth, f.bits_per_pixel, f.scanline_pad
        );
    }

    let _ = writeln!(o, "\n[screen]");
    let _ = writeln!(
        o,
        "{}x{} depth={} root_visual=0x{:x} white=0x{:x} black=0x{:x}",
        screen.width_in_pixels,
        screen.height_in_pixels,
        screen.root_depth,
        screen.root_visual,
        screen.white_pixel,
        screen.black_pixel
    );
    for d in &screen.allowed_depths {
        for v in &d.visuals {
            let _ = writeln!(
                o,
                "depth={} visual=0x{:x} class={:?} masks r=0x{:x} g=0x{:x} b=0x{:x}",
                d.depth, v.visual_id, v.class, v.red_mask, v.green_mask, v.blue_mask
            );
        }
    }

    // A lab126 or eink-specific name here is a refresh mechanism.
    let _ = writeln!(o, "\n[extensions]");
    match conn
        .list_extensions()
        .map_err(|e| e.to_string())
        .and_then(|c| c.reply().map_err(|e| e.to_string()))
    {
        Ok(r) => {
            let mut names: Vec<String> = r
                .names
                .iter()
                .map(|n| String::from_utf8_lossy(&n.name).into_owned())
                .collect();
            names.sort();
            for n in &names {
                let _ = writeln!(o, "{n}");
            }
        }
        Err(e) => {
            let _ = writeln!(o, "ListExtensions failed: {e}");
        }
    }

    probe_paint(&conn, &screen, o);
}

/// Paint in bands and time each one's round trip. Every band reports its own
/// byte count, elapsed time and error.
fn probe_paint(conn: &impl Connection, screen: &x11rb::protocol::xproto::Screen, o: &mut String) {
    let _ = writeln!(o, "\n[paint test]");
    let xres = screen.width_in_pixels as usize;
    let yres = screen.height_in_pixels as usize;
    let depth = screen.root_depth;
    let bpp = conn
        .setup()
        .pixmap_formats
        .iter()
        .find(|f| f.depth == depth)
        .map(|f| (f.bits_per_pixel as usize / 8).max(1))
        .unwrap_or(1);

    let win = match conn.generate_id() {
        Ok(w) => w,
        Err(e) => {
            let _ = writeln!(o, "generate_id failed: {e}");
            return;
        }
    };
    if let Err(e) = conn.create_window(
        depth,
        win,
        screen.root,
        0,
        0,
        screen.width_in_pixels,
        screen.height_in_pixels,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new()
            .background_pixel(screen.white_pixel)
            .event_mask(EventMask::EXPOSURE),
    ) {
        let _ = writeln!(o, "create_window failed: {e}");
        return;
    }
    let name = b"L:A_N:application_ID:com.kfxdedrmfe.probe_PC:N_O:U";
    let _ = conn.change_property8(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        name,
    );
    let _ = conn.map_window(win);
    let _ = conn.flush();

    match conn
        .get_geometry(win)
        .map_err(|e| e.to_string())
        .and_then(|c| c.reply().map_err(|e| e.to_string()))
    {
        Ok(g) => {
            let _ = writeln!(
                o,
                "window geometry {}x{}+{}+{} (root {}x{})",
                g.width, g.height, g.x, g.y, screen.width_in_pixels, screen.height_in_pixels
            );
        }
        Err(e) => {
            let _ = writeln!(o, "get_geometry failed: {e}");
        }
    }

    let gc = match conn.generate_id() {
        Ok(g) => g,
        Err(e) => {
            let _ = writeln!(o, "generate_id gc failed: {e}");
            return;
        }
    };
    let _ = conn.create_gc(gc, win, &CreateGCAux::new());

    // Alternating black and white bands, read off the panel by eye.
    for (label, limit) in [
        ("256KB", 256 * 1024usize),
        ("1MB", 1024 * 1024),
        ("full-screen", xres * yres * bpp + 1024),
    ] {
        let stride = xres * bpp;
        let max_rows = (limit.saturating_sub(64) / stride.max(1)).max(1).min(yres);
        let bands = yres.div_ceil(max_rows);
        let t0 = Instant::now();
        let mut failed = 0;
        let mut y = 0usize;
        let mut shade = 0x00u8;
        while y < yres {
            let h = (yres - y).min(max_rows);
            let buf = vec![shade; h * stride];
            shade = !shade;
            if let Err(e) = conn.put_image(
                ImageFormat::Z_PIXMAP,
                win,
                gc,
                xres as u16,
                h as u16,
                0,
                y as i16,
                0,
                depth,
                &buf,
            ) {
                failed += 1;
                let _ = writeln!(o, "  {label}: put_image row {y} send error: {e}");
            }
            y += h;
        }
        // The reply lands once the server has processed every request above.
        let sync = conn
            .get_input_focus()
            .map_err(|e| e.to_string())
            .and_then(|c| c.reply().map_err(|e| e.to_string()));
        let elapsed = t0.elapsed();
        let _ = writeln!(
            o,
            "  {label}: {bands} band(s) of {max_rows} rows, send errors={failed}, \
             round-trip {elapsed:?}, sync={}",
            match &sync {
                Ok(_) => "ok".to_string(),
                Err(e) => e.to_string(),
            }
        );
        // Asynchronous protocol errors land here.
        while let Ok(Some(ev)) = conn.poll_for_event() {
            if let x11rb::protocol::Event::Error(e) = ev {
                let _ = writeln!(o, "  {label}: X error {e:?}");
            }
        }
    }

    probe_supersession(conn, screen, win, gc, xres, yres, bpp, depth, o);

    let _ = conn.destroy_window(win);
    let _ = conn.flush();
}

/// Paint full-screen against a burst of small updates, in three phases. X
/// reports success in each; the panel is the readout, and every phase prints
/// what it painted.
#[allow(clippy::too_many_arguments)]
fn probe_supersession(
    conn: &impl Connection,
    _screen: &x11rb::protocol::xproto::Screen,
    win: x11rb::protocol::xproto::Window,
    gc: x11rb::protocol::xproto::Gcontext,
    xres: usize,
    yres: usize,
    bpp: usize,
    depth: u8,
    o: &mut String,
) {
    let stride = xres * bpp;
    let full = |shade: u8| vec![shade; yres * stride];

    let paint_full = |shade: u8| {
        let buf = full(shade);
        let _ = conn.put_image(
            ImageFormat::Z_PIXMAP,
            win,
            gc,
            xres as u16,
            yres as u16,
            0,
            0,
            0,
            depth,
            &buf,
        );
    };

    // 24 small squares scattered down the screen, mimicking the per-cover burst.
    let paint_squares = |shade: u8| {
        const S: usize = 120;
        for i in 0..24usize {
            let y = (i * (yres.saturating_sub(S)) / 24).min(yres.saturating_sub(S));
            let x = (i % 4) * S * 2;
            if x + S > xres {
                continue;
            }
            let buf = vec![shade; S * S * bpp];
            let _ = conn.put_image(
                ImageFormat::Z_PIXMAP,
                win,
                gc,
                S as u16,
                S as u16,
                x as i16,
                y as i16,
                0,
                depth,
                &buf,
            );
        }
    };

    let sync = |conn: &dyn Fn()| conn();
    let _ = sync;

    let _ = writeln!(o, "\n[supersession test]");
    let _ = writeln!(
        o,
        "Watch the panel. Each phase pauses 3s so you can see the result."
    );

    // Phase 1 — a full paint with nothing competing.
    let t = Instant::now();
    paint_full(0x00);
    let _ = conn
        .get_input_focus()
        .map_err(|e| e.to_string())
        .and_then(|c| c.reply().map(|_| ()).map_err(|e| e.to_string()));
    let _ = writeln!(
        o,
        "  phase 1 (control): full BLACK, no competing updates, ack {:?}\n\
         \x20   EXPECT: the whole screen goes black.",
        t.elapsed()
    );
    std::thread::sleep(std::time::Duration::from_secs(3));

    // Phase 2 — a full paint immediately followed by the burst.
    let t = Instant::now();
    paint_full(0xFF);
    paint_squares(0x00);
    let _ = conn
        .get_input_focus()
        .map_err(|e| e.to_string())
        .and_then(|c| c.reply().map(|_| ()).map_err(|e| e.to_string()));
    let _ = writeln!(
        o,
        "  phase 2 (contended): full WHITE then 24 black squares, ack {:?}\n\
         \x20   EXPECT if refreshes are NOT superseded: white screen + black squares.\n\
         \x20   IF SUPERSEDED: still mostly BLACK from phase 1, with black squares\n\
         \x20   barely distinguishable — i.e. the white never arrived.",
        t.elapsed()
    );
    std::thread::sleep(std::time::Duration::from_secs(3));

    // Phase 3 — the same content, with a settle ahead of the partials.
    let t = Instant::now();
    paint_full(0xFF);
    let _ = conn
        .get_input_focus()
        .map_err(|e| e.to_string())
        .and_then(|c| c.reply().map(|_| ()).map_err(|e| e.to_string()));
    std::thread::sleep(std::time::Duration::from_millis(1200));
    paint_squares(0x00);
    let _ = conn
        .get_input_focus()
        .map_err(|e| e.to_string())
        .and_then(|c| c.reply().map(|_| ()).map_err(|e| e.to_string()));
    let _ = writeln!(
        o,
        "  phase 3 (spaced): full WHITE, wait 1.2s, then the squares, ack {:?}\n\
         \x20   EXPECT: white screen + black squares. If phase 2 failed and this\n\
         \x20   works, a settle delay after a full paint is the fix.",
        t.elapsed()
    );
    std::thread::sleep(std::time::Duration::from_secs(3));

    while let Ok(Some(ev)) = conn.poll_for_event() {
        if let x11rb::protocol::Event::Error(e) = ev {
            let _ = writeln!(o, "  supersession: X error {e:?}");
        }
    }
}

/// Paint the same content into two windows, one with `BackingStore::ALWAYS`
/// and one without.
fn probe_backing_store(o: &mut String) {
    let _ = writeln!(o, "\n[backing-store test]");
    let (conn, screen_num) = match x11rb::connect(None) {
        Ok(c) => c,
        Err(e) => {
            let _ = writeln!(o, "connect FAILED: {e}");
            return;
        }
    };
    let screen = conn.setup().roots[screen_num].clone();
    let xres = screen.width_in_pixels as usize;
    let yres = screen.height_in_pixels as usize;
    let depth = screen.root_depth;
    let bpp = conn
        .setup()
        .pixmap_formats
        .iter()
        .find(|f| f.depth == depth)
        .map(|f| (f.bits_per_pixel as usize / 8).max(1))
        .unwrap_or(1);

    for (label, backing) in [
        ("WITHOUT backing store", false),
        ("WITH backing store", true),
    ] {
        let Ok(win) = conn.generate_id() else {
            continue;
        };
        let mut aux = CreateWindowAux::new()
            .background_pixel(screen.white_pixel)
            .event_mask(EventMask::EXPOSURE);
        if backing {
            aux = aux.backing_store(BackingStore::ALWAYS);
        }
        if let Err(e) = conn.create_window(
            depth,
            win,
            screen.root,
            0,
            0,
            screen.width_in_pixels,
            screen.height_in_pixels,
            0,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &aux,
        ) {
            let _ = writeln!(o, "  {label}: create_window failed: {e}");
            continue;
        }
        let name = b"L:A_N:application_ID:com.kfxdedrmfe.probe_PC:N_O:U";
        let _ = conn.change_property8(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            name,
        );
        let _ = conn.map_window(win);
        let _ = conn.flush();
        let Ok(gc) = conn.generate_id() else { continue };
        let _ = conn.create_gc(gc, win, &CreateGCAux::new());

        let sync = |conn: &x11rb::rust_connection::RustConnection| {
            let _ = conn
                .get_input_focus()
                .map_err(|e| e.to_string())
                .and_then(|c| c.reply().map(|_| ()).map_err(|e| e.to_string()));
        };
        let put = |shade: u8, w: usize, h: usize, x: i16, y: i16| {
            let buf = vec![shade; w * h * bpp];
            let _ = conn.put_image(
                ImageFormat::Z_PIXMAP,
                win,
                gc,
                w as u16,
                h as u16,
                x,
                y,
                0,
                depth,
                &buf,
            );
        };

        // Black, then white with squares.
        put(0x00, xres, yres, 0, 0);
        sync(&conn);
        std::thread::sleep(std::time::Duration::from_secs(2));
        let t = Instant::now();
        put(0xFF, xres, yres, 0, 0);
        for i in 0..24usize {
            let s = 120usize;
            let y = (i * yres.saturating_sub(s) / 24).min(yres.saturating_sub(s));
            let x = (i % 4) * s * 2;
            if x + s <= xres {
                put(0x00, s, s, x as i16, y as i16);
            }
        }
        sync(&conn);
        let _ = writeln!(
            o,
            "  {label}: painted BLACK, then WHITE + 24 squares (ack {:?})\n\
             \x20   EXPECT: white screen with black squares.\n\
             \x20   If this one stays BLACK while the other was correct, backing\n\
             \x20   store is the cause.",
            t.elapsed()
        );
        std::thread::sleep(std::time::Duration::from_secs(3));

        while let Ok(Some(ev)) = conn.poll_for_event() {
            if let x11rb::protocol::Event::Error(e) = ev {
                let _ = writeln!(o, "  {label}: X error {e:?}");
            }
        }
        let _ = conn.destroy_window(win);
        let _ = conn.flush();
    }
}

/// Paint known squares, read `/dev/fb0` back, and print per-square coverage
/// with the dark rows of each. A stride error drifts those rows sideways; a
/// dropped update cuts a square on a clean horizontal edge.
fn probe_fb_readback(o: &mut String) {
    let _ = writeln!(o, "\n[framebuffer readback]");

    let (fb_w, fb_h) = match read_virtual_size() {
        Some(v) => v,
        None => {
            let _ = writeln!(o, "could not read virtual_size; skipping");
            return;
        }
    };
    let fb_bpp = std::fs::read_to_string("/sys/class/graphics/fb0/bits_per_pixel")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .map(|b| (b / 8).max(1))
        .unwrap_or(1);
    let _ = writeln!(o, "fb stride={fb_w}px virtual_h={fb_h} bytes/px={fb_bpp}");

    let (conn, screen_num) = match x11rb::connect(None) {
        Ok(c) => c,
        Err(e) => {
            let _ = writeln!(o, "connect FAILED: {e}");
            return;
        }
    };
    let screen = conn.setup().roots[screen_num].clone();
    let xres = screen.width_in_pixels as usize;
    let yres = screen.height_in_pixels as usize;
    let depth = screen.root_depth;
    let bpp = conn
        .setup()
        .pixmap_formats
        .iter()
        .find(|f| f.depth == depth)
        .map(|f| (f.bits_per_pixel as usize / 8).max(1))
        .unwrap_or(1);
    if xres != fb_w {
        let _ = writeln!(
            o,
            "NOTE X width {xres} != fb stride {fb_w} — a shear, if present, is {} px/row",
            fb_w as i64 - xres as i64
        );
    }

    let Ok(win) = conn.generate_id() else { return };
    if conn
        .create_window(
            depth,
            win,
            screen.root,
            0,
            0,
            screen.width_in_pixels,
            screen.height_in_pixels,
            0,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &CreateWindowAux::new()
                .background_pixel(screen.white_pixel)
                .event_mask(EventMask::EXPOSURE),
        )
        .is_err()
    {
        return;
    }
    let name = b"L:A_N:application_ID:com.kfxdedrmfe.probe_PC:N_O:U";
    let _ = conn.change_property8(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        name,
    );
    let _ = conn.map_window(win);
    let Ok(gc) = conn.generate_id() else { return };
    let _ = conn.create_gc(gc, win, &CreateGCAux::new());
    let sync = || {
        let _ = conn
            .get_input_focus()
            .map_err(|e| e.to_string())
            .and_then(|c| c.reply().map(|_| ()).map_err(|e| e.to_string()));
    };

    // A white field, then squares on a known grid.
    const S: usize = 120;
    let squares: Vec<(usize, usize)> = (0..24)
        .map(|i| {
            let col = i % 4;
            let row = i / 4;
            (200 + col * 380, 100 + row * 380)
        })
        .filter(|(x, y)| x + S <= xres && y + S <= yres)
        .collect();

    let white = vec![0xFFu8; yres * xres * bpp];
    let _ = conn.put_image(
        ImageFormat::Z_PIXMAP,
        win,
        gc,
        xres as u16,
        yres as u16,
        0,
        0,
        0,
        depth,
        &white,
    );
    sync();
    std::thread::sleep(std::time::Duration::from_millis(1500));

    let black = vec![0x00u8; S * S * bpp];
    for (x, y) in &squares {
        let _ = conn.put_image(
            ImageFormat::Z_PIXMAP,
            win,
            gc,
            S as u16,
            S as u16,
            *x as i16,
            *y as i16,
            0,
            depth,
            &black,
        );
    }
    sync();
    std::thread::sleep(std::time::Duration::from_millis(1500));

    // Both pages: virtual height is 2x and the live one is not exposed.
    let page_bytes = fb_w * yres * fb_bpp;
    for page in 0..2usize {
        let Some(buf) = read_fb_page(page * page_bytes, page_bytes) else {
            let _ = writeln!(o, "page {page}: unreadable");
            continue;
        };
        let dark = |x: usize, y: usize| -> bool {
            let idx = y * fb_w * fb_bpp + x * fb_bpp;
            buf.get(idx).is_some_and(|v| *v < 0x40)
        };
        // Coverage per square, plus which of its rows are dark.
        let mut summary = Vec::new();
        for (i, (x, y)) in squares.iter().enumerate() {
            let mut dark_px = 0usize;
            let mut first_bad_row = None;
            for dy in 0..S {
                let row_dark = (0..S).filter(|dx| dark(x + dx, y + dy)).count();
                dark_px += row_dark;
                if row_dark < S / 2 && first_bad_row.is_none() {
                    first_bad_row = Some(dy);
                }
            }
            let pct = dark_px * 100 / (S * S);
            summary.push((i, pct, first_bad_row));
        }
        let complete = summary.iter().filter(|(_, p, _)| *p >= 95).count();
        let empty = summary.iter().filter(|(_, p, _)| *p <= 5).count();
        let partial = summary.len() - complete - empty;
        let _ = writeln!(
            o,
            "page {page}: {complete} complete, {partial} partial, {empty} missing (of {})",
            summary.len()
        );
        for (i, pct, bad) in &summary {
            if *pct < 95 {
                let _ = writeln!(
                    o,
                    "  square {i} at ({},{}) coverage {pct}% first-bad-row {:?}",
                    squares[*i].0, squares[*i].1, bad
                );
            }
        }
    }

    let _ = conn.destroy_window(win);
    let _ = conn.flush();
}

fn read_virtual_size() -> Option<(usize, usize)> {
    let s = std::fs::read_to_string("/sys/class/graphics/fb0/virtual_size").ok()?;
    let (w, h) = s.trim().split_once(',')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

fn read_fb_page(offset: usize, len: usize) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open("/dev/fb0").ok()?;
    f.seek(SeekFrom::Start(offset as u64)).ok()?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// The pre-X refresh paths: which exist, and what each reports.
fn probe_eink_paths(o: &mut String) {
    let _ = writeln!(o, "\n[eink control paths]");
    for p in [
        "/dev/fb0",
        "/dev/mxc_epdc_fb",
        "/proc/eink_fb",
        "/proc/eink_fb/update_display",
        "/sys/class/graphics/fb0",
        "/sys/class/graphics/fb0/epd_update",
        "/usr/bin/eips",
        "/usr/sbin/eips",
        "/usr/bin/lipc-set-prop",
    ] {
        let _ = writeln!(
            o,
            "{p}: {}",
            if Path::new(p).exists() {
                "present"
            } else {
                "absent"
            }
        );
    }
    // fb0's own geometry, separate from what X reports.
    for p in [
        "/sys/class/graphics/fb0/virtual_size",
        "/sys/class/graphics/fb0/bits_per_pixel",
        "/sys/class/graphics/fb0/name",
    ] {
        if let Ok(v) = std::fs::read_to_string(p) {
            let _ = writeln!(o, "{p} = {}", v.trim());
        }
    }
    if let Ok(entries) = std::fs::read_dir("/proc/eink_fb") {
        for e in entries.flatten() {
            let _ = writeln!(o, "/proc/eink_fb/{}", e.file_name().to_string_lossy());
        }
    }
}

/// Best-effort `uname -a` and friends, identifying the device.
pub fn device_line() -> String {
    std::fs::read_to_string("/proc/version").unwrap_or_else(|_| "unknown".into())
}

pub fn run_logged() -> Result<()> {
    println!("{}", device_line().trim());
    run().context("x probe")
}
