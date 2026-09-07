//! [`Framebuffer`] — a fullscreen X11 window over a packed-RGB backing at
//! [`CH`] bytes per pixel, with [`Framebuffer::send_update`] presenting dirty
//! rows in the server's wire format at identity. `_waveform` is ignored.
//!
//! The window is WM-managed, so the size it is laid out at is the WM's answer
//! and not the root's: [`Framebuffer::open`] waits for `MapNotify` and takes
//! `get_geometry`'s, and [`Framebuffer::pump_events`] follows a later
//! `ConfigureNotify`. `var`, `backing` and `wire_stride` move together.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use x11rb::connection::Connection;
// `maximum_request_bytes` (BIG-REQUESTS-aware) lives on this trait.
use x11rb::connection::RequestConnection as _;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt, CreateGCAux, CreateWindowAux, EventMask, Gcontext, ImageFormat,
    ImageOrder, PropMode, Screen, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
// `change_property8` lives in the wrapper `ConnectionExt`.
use x11rb::wrapper::ConnectionExt as _;

// `send_update` accepts and ignores these.
#[allow(dead_code)]
pub const WAVEFORM_MODE_INIT: u32 = 0;
pub const WAVEFORM_MODE_DU: u32 = 1;
pub const WAVEFORM_MODE_GC16: u32 = 2;

/// Bytes per pixel in `backing`: packed RGB, no alpha.
pub const CH: usize = 3;

/// Rec. 601 luma of `r`, `g`, `b`. 77, 150 and 29 sum to 256.
#[inline]
fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((r as u32 * 77 + g as u32 * 150 + b as u32 * 29) >> 8) as u8
}

/// The R, G and B byte offsets within a `bpp`-wide wire pixel, from
/// `screen.root_visual`'s colour masks under `conn`'s image byte order.
/// `None` for `bpp < 3`, an unreadable visual, or a zero mask.
fn wire_channels(conn: &RustConnection, screen: &Screen, bpp: usize) -> Option<[usize; 3]> {
    if bpp < 3 {
        return None;
    }
    let visual = screen
        .allowed_depths
        .iter()
        .flat_map(|d| d.visuals.iter())
        .find(|v| v.visual_id == screen.root_visual)?;
    if visual.red_mask == 0 || visual.green_mask == 0 || visual.blue_mask == 0 {
        return None;
    }
    let msb = conn.setup().image_byte_order == ImageOrder::MSB_FIRST;
    // `mask` sits in one byte of the native-endian pixel; `idx` is its
    // trailing-zero count / 8, mirrored across `bpp` under `msb`.
    let offset = |mask: u32| -> usize {
        let idx = (mask.trailing_zeros() / 8) as usize;
        if msb { bpp - 1 - idx } else { idx }
    };
    Some([
        offset(visual.red_mask),
        offset(visual.green_mask),
        offset(visual.blue_mask),
    ])
}

/// `events` folded into a [`Pump`] against the `size` being drawn. The last
/// `ConfigureNotify` wins, and a layout matching `size` answers `None`.
fn fold(events: &[Event], size: (u32, u32)) -> Pump {
    let mut pump = Pump::default();
    for event in events {
        match event {
            Event::Expose(_) => pump.repaint = true,
            // `ConfigureNotify` carries the size the window is laid out at.
            Event::ConfigureNotify(ev) => {
                let laid = (u32::from(ev.width), u32::from(ev.height));
                pump.resized = (laid != size && laid.0 > 0 && laid.1 > 0).then_some(laid);
            }
            Event::Error(e) => {
                // A `put_image` the server rejects arrives here.
                eprintln!("x11: WARNING request failed: {e:?}");
                pump.repaint = true;
            }
            _ => {}
        }
    }
    pump
}

/// `pixel_bytes` rounded up to a multiple of `pad`, the bytes `put_image` takes
/// per ZPixmap scanline.
fn wire_stride(pixel_bytes: usize, pad: usize) -> usize {
    let pad = pad.max(1);
    pixel_bytes.div_ceil(pad) * pad
}

/// `band`'s packed-RGB rows, `xres` wide, into `wire`: one `bpp`-byte wire
/// pixel each, rows `wire_stride` apart with the pad left at 0xFF. `bpp == 1`
/// collapses to one luma byte; wider scatters R/G/B to `chan`.
fn pack_band(
    wire: &mut Vec<u8>,
    band: &[u8],
    xres: usize,
    bpp: usize,
    wire_stride: usize,
    chan: [usize; 3],
) {
    wire.clear();
    let bk_stride = xres * CH;
    if bk_stride == 0 || wire_stride == 0 {
        return;
    }
    let pixel_bytes = xres * bpp;
    let [rb, gb, bb] = chan;
    wire.resize(band.len() / bk_stride * wire_stride, 0xFF);
    for (out, row) in wire
        .chunks_exact_mut(wire_stride)
        .zip(band.chunks_exact(bk_stride))
    {
        let (triples, _) = row.as_chunks::<CH>();
        let pairs = out[..pixel_bytes].chunks_exact_mut(bpp).zip(triples);
        if bpp == 1 {
            for (w, rgb) in pairs {
                w[0] = luma(rgb[0], rgb[1], rgb[2]);
            }
        } else {
            for (w, rgb) in pairs {
                w[rb] = rgb[0];
                w[gb] = rgb[1];
                w[bb] = rgb[2];
            }
        }
    }
}

/// A rectangle to present, in screen coords.
#[derive(Default, Debug, Clone, Copy)]
pub struct MxcfbRect {
    pub top: u32,
    #[allow(dead_code)]
    pub left: u32,
    #[allow(dead_code)]
    pub width: u32,
    pub height: u32,
}

/// Minimal geometry, exposed as `fb.var.xres` / `fb.var.yres`.
pub struct Var {
    pub xres: u32,
    pub yres: u32,
}

/// What [`Framebuffer::pump_events`] answers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pump {
    /// Set by `Expose` and by `Event::Error`.
    pub repaint: bool,
    /// A `ConfigureNotify` size differing from the one being drawn.
    pub resized: Option<(u32, u32)>,
}

/// How long [`Framebuffer::open`] waits for `MapNotify`.
const LAYOUT_WAIT: Duration = Duration::from_millis(500);

pub struct Framebuffer {
    conn: RustConnection,
    win: Window,
    gc: Gcontext,
    depth: u8,
    /// Wire bytes per pixel for `depth`: 1 at depth 8, 4 at depth 24/32.
    bytes_per_pixel: usize,
    /// Wire bytes per scanline: [`wire_stride`] over `xres` and the format's
    /// `scanline_pad`.
    wire_stride: usize,
    /// The format's `scanline_pad`, in bytes: [`wire_stride`]'s second half.
    scanline_pad: usize,
    /// `wire_channels`' R, G, B byte offsets. Depth-24 LSB-first is `[2, 1, 0]`.
    chan: [usize; 3],
    pub var: Var,
    /// Packed RGB at [`CH`] bytes/pixel, stride `xres * CH`.
    backing: Vec<u8>,
    /// Per-`PutImage` byte budget (server max request length minus header slack).
    max_req_bytes: usize,
}

impl Framebuffer {
    /// Connect to the X server (`$DISPLAY`), create + map a fullscreen window.
    pub fn open() -> Result<Self> {
        let (conn, screen_num) = x11rb::connect(None).context("connect to X ($DISPLAY)")?;
        let screen = conn.setup().roots[screen_num].clone();
        // The root size, until `get_geometry` answers below.
        let mut xres = screen.width_in_pixels as u32;
        let mut yres = screen.height_in_pixels as u32;
        let depth = screen.root_depth;
        let format = conn
            .setup()
            .pixmap_formats
            .iter()
            .find(|f| f.depth == depth);
        // `bits_per_pixel` for `depth`: 8 → 1 byte, 24/32 → 4 bytes.
        let bytes_per_pixel = format
            .map(|f| (f.bits_per_pixel as usize / 8).max(1))
            .unwrap_or(1);
        // `scanline_pad` is 32 bits on every standard format.
        let scanline_pad = format.map(|f| f.scanline_pad as usize / 8).unwrap_or(4);
        // `[2, 1, 0]` is BGRX LSB-first, the lab126 depth-24 layout.
        let chan = wire_channels(&conn, &screen, bytes_per_pixel).unwrap_or([2, 1, 0]);
        eprintln!(
            "fb: root {xres}x{yres} depth={depth} bytes_per_pixel={bytes_per_pixel} \
             scanline_pad={scanline_pad} \
             chan=[{},{},{}] root_visual=0x{:x}",
            chan[0], chan[1], chan[2], screen.root_visual,
        );

        let win = conn.generate_id().context("generate_id window")?;
        conn.create_window(
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
            // `CreateWindowAux` sets no `backing_store`.
            &CreateWindowAux::new()
                .background_pixel(screen.white_pixel)
                // `STRUCTURE_NOTIFY` carries `MapNotify` and `ConfigureNotify`;
                // the WM lays this window out and may not hand it the root size.
                .event_mask(EventMask::EXPOSURE | EventMask::STRUCTURE_NOTIFY),
        )
        .context("create_window")?;

        // `WM_NAME` is a lab126 layout spec: application layer, no chrome,
        // fullscreen.
        let name = b"L:A_N:application_ID:com.kfxdedrmfe.picker_PC:N_O:U";
        conn.change_property8(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            name,
        )
        .context("set WM_NAME")?;

        conn.map_window(win).context("map_window")?;

        let gc = conn.generate_id().context("generate_id gc")?;
        conn.create_gc(gc, win, &CreateGCAux::new())
            .context("create_gc")?;
        conn.flush().context("flush after map")?;

        // `MapNotify` marks the layout done.
        let deadline = Instant::now() + LAYOUT_WAIT;
        let mut mapped = false;
        while !mapped && Instant::now() < deadline {
            while let Ok(Some(event)) = conn.poll_for_event() {
                mapped |= matches!(event, Event::MapNotify(_));
            }
            if !mapped {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        eprintln!("fb: mapped={mapped}");

        // `get_geometry` now that the WM has laid the window out.
        match conn
            .get_geometry(win)
            .map_err(|e| e.to_string())
            .and_then(|c| c.reply().map_err(|e| e.to_string()))
        {
            Ok(g) if u32::from(g.width) != xres || u32::from(g.height) != yres => {
                // `get_geometry` outranks the root read above: the UI is drawn
                // at the size the window was laid out at, not the panel's.
                eprintln!(
                    "fb: window is {}x{}, root read {xres}x{yres}",
                    g.width, g.height
                );
                (xres, yres) = (u32::from(g.width), u32::from(g.height));
            }
            Ok(_) => eprintln!("fb: window geometry matches root ({xres}x{yres})"),
            Err(e) => eprintln!("fb: could not read window geometry: {e}"),
        }

        // `wire_stride` follows `xres`, which `get_geometry` above sets.
        let wire_stride = wire_stride(xres as usize * bytes_per_pixel, scanline_pad);
        eprintln!("fb: drawing {xres}x{yres}, wire_stride={wire_stride}");

        // `maximum_request_bytes` is the post-BIG-REQUESTS limit, ~16 MB;
        // `setup().maximum_request_length` is the ~256 KB pre-negotiation one.
        let max_req_bytes = conn.maximum_request_bytes().max(4096);
        eprintln!(
            "fb: max request {} bytes ({} rows/band at {} bpp)",
            max_req_bytes,
            max_req_bytes / wire_stride.max(1),
            bytes_per_pixel
        );

        let backing = vec![0xFFu8; xres as usize * yres as usize * CH];

        Ok(Self {
            conn,
            win,
            gc,
            depth,
            bytes_per_pixel,
            wire_stride,
            scanline_pad,
            chan,
            var: Var { xres, yres },
            backing,
            max_req_bytes,
        })
    }

    /// Single gray-pixel write in screen coords (0=black, 255=white), stored as
    /// `(v,v,v)`, no-op out of range.
    #[inline]
    pub fn put_pixel(&mut self, x: i32, y: i32, value: u8) {
        self.put_pixel_rgb(x, y, [value, value, value]);
    }

    /// Single color-pixel write in screen coords, `rgb`, no-op out of range.
    #[inline]
    pub fn put_pixel_rgb(&mut self, x: i32, y: i32, rgb: [u8; 3]) {
        if x < 0 || y < 0 || x >= self.var.xres as i32 || y >= self.var.yres as i32 {
            return;
        }
        let idx = (y as usize * self.var.xres as usize + x as usize) * CH;
        if idx + CH <= self.backing.len() {
            self.backing[idx..idx + CH].copy_from_slice(&rgb);
        }
    }

    /// Fill a rectangle with gray `value` (0=black, 255=white). Every `backing`
    /// byte in the span takes `value`.
    pub fn fill_rect(&mut self, top: u32, left: u32, width: u32, height: u32, value: u8) {
        if left >= self.var.xres {
            return;
        }
        let stride = self.var.xres as usize * CH;
        let max_y = top.saturating_add(height).min(self.var.yres);
        let max_x = left.saturating_add(width).min(self.var.xres);
        for y in top..max_y {
            let row = y as usize * stride;
            let s = row + left as usize * CH;
            let e = row + max_x as usize * CH;
            if e <= self.backing.len() {
                self.backing[s..e].fill(value);
            }
        }
    }

    /// Drains the X event queue through [`fold`], resizing on a
    /// `Pump::resized`. `put_image` returns a `VoidCookie`; a request the
    /// server rejects arrives here as `Event::Error`.
    ///
    /// Nothing else reads the queue, so a loop that blocks without calling
    /// this notices neither an `Expose` nor a relayout.
    pub fn pump_events(&mut self) -> Pump {
        let size = (self.var.xres, self.var.yres);
        let mut events = Vec::new();
        while let Ok(Some(event)) = self.conn.poll_for_event() {
            events.push(event);
        }
        let pump = fold(&events, size);
        if let Some((w, h)) = pump.resized {
            self.resize(w, h);
        }
        pump
    }

    /// Draws `w` by `h`: `var`, `backing` and `wire_stride` follow, and
    /// `backing` is left white for the caller's full repaint.
    fn resize(&mut self, w: u32, h: u32) {
        eprintln!(
            "fb: laid out {w}x{h}, was {}x{}",
            self.var.xres, self.var.yres
        );
        self.var = Var { xres: w, yres: h };
        self.backing = vec![0xFFu8; w as usize * h as usize * CH];
        self.wire_stride = wire_stride(w as usize * self.bytes_per_pixel, self.scanline_pad);
    }

    /// Present rows `rect.top..rect.top + rect.height`, a band at a time through
    /// [`pack_band`]. `_waveform` is ignored.
    pub fn send_update(&mut self, rect: MxcfbRect, _waveform: u32) -> Result<u32> {
        let bpp = self.bytes_per_pixel;
        let xres = self.var.xres as usize;
        let bk_stride = xres * CH; // backing bytes per scanline (RGB)
        let wire_stride = self.wire_stride; // wire bytes per scanline, padded
        let width = self.var.xres as u16;
        let top = rect.top.min(self.var.yres);
        let bottom = rect.top.saturating_add(rect.height).min(self.var.yres);
        let max_rows = (self.max_req_bytes.saturating_sub(64) / wire_stride.max(1)).max(1);

        // `wire` is reused across bands.
        let mut wire: Vec<u8> = Vec::new();

        let mut y = top;
        while y < bottom {
            let h = ((bottom - y) as usize).min(max_rows);
            let s = y as usize * bk_stride;
            let e = s + h * bk_stride;
            pack_band(
                &mut wire,
                &self.backing[s..e],
                xres,
                bpp,
                wire_stride,
                self.chan,
            );

            self.conn
                .put_image(
                    ImageFormat::Z_PIXMAP,
                    self.win,
                    self.gc,
                    width,
                    h as u16,
                    0,
                    y as i16,
                    0,
                    self.depth,
                    &wire,
                )
                .context("put_image")?;
            y += h as u32;
        }
        // `get_input_focus` round-trips: its reply lands once the server has
        // processed every preceding `put_image`.
        self.conn
            .get_input_focus()
            .context("sync round-trip")?
            .reply()
            .context("sync reply")?;
        Ok(0)
    }

    /// Clone `backing`.
    pub fn backing_snapshot(&self) -> Vec<u8> {
        self.backing.clone()
    }

    /// Replace `backing` with `snap`, no-op on a length mismatch.
    pub fn restore_backing(&mut self, snap: Vec<u8>) {
        if snap.len() == self.backing.len() {
            self.backing = snap;
        }
    }

    /// Encode `backing` (packed RGB, white=255) as a PNG at `path`, unrotated.
    pub fn capture_png(&self, path: &Path) -> Result<()> {
        let img = image::RgbImage::from_raw(self.var.xres, self.var.yres, self.backing.clone())
            .context("backing buffer size != xres*yres*CH")?;
        img.save(path)
            .with_context(|| format!("write screenshot {}", path.display()))?;
        Ok(())
    }
}

impl Drop for Framebuffer {
    fn drop(&mut self) {
        // `destroy_window` hands the screen back to the WM.
        let _ = self.conn.destroy_window(self.win);
        let _ = self.conn.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::{Pump, fold, pack_band, wire_stride};
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::{ConfigureNotifyEvent, ExposeEvent};

    /// The size being drawn, as `pump_events` passes it.
    const SIZE: (u32, u32) = (1264, 1680);

    /// A `ConfigureNotify` laying the window out at `w` by `h`.
    fn configure(w: u16, h: u16) -> Event {
        Event::ConfigureNotify(ConfigureNotifyEvent {
            response_type: 22,
            sequence: 0,
            event: 1,
            window: 1,
            above_sibling: 0,
            x: 0,
            y: 0,
            width: w,
            height: h,
            border_width: 0,
            override_redirect: false,
        })
    }

    fn expose() -> Event {
        Event::Expose(ExposeEvent {
            response_type: 12,
            sequence: 0,
            window: 1,
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            count: 0,
        })
    }

    /// An empty queue asks for nothing; an `Expose` asks for a repaint alone.
    #[test]
    fn nothing_drained_changes_nothing() {
        assert_eq!(fold(&[], SIZE), Pump::default());
        let pump = fold(&[expose()], SIZE);
        assert!(pump.repaint);
        assert_eq!(pump.resized, None);
    }

    /// A layout differing from `size` is reported, and one matching it is not.
    /// A zero-sided layout is not a size to draw at.
    #[test]
    fn only_a_new_layout_is_reported() {
        assert_eq!(
            fold(&[configure(1680, 1264)], SIZE).resized,
            Some((1680, 1264))
        );
        assert_eq!(fold(&[configure(1264, 1680)], SIZE).resized, None);
        assert_eq!(fold(&[configure(0, 0)], SIZE).resized, None);
    }

    /// The last `ConfigureNotify` of a drain wins, so a drain ending back at
    /// `size` reports nothing.
    #[test]
    fn the_last_layout_of_a_drain_wins() {
        let events = [configure(1680, 1264), configure(1264, 1680)];
        assert_eq!(fold(&events, SIZE).resized, None);
        let events = [configure(1264, 1680), configure(1680, 1264)];
        assert_eq!(fold(&events, SIZE).resized, Some((1680, 1264)));
    }

    /// `wire_stride` over the shipped panel widths at both `bytes_per_pixel` a
    /// Kindle X server offers, under a 4-byte pad.
    #[test]
    fn a_scanline_reaches_the_pad() {
        // 758 is the one width that is not itself a multiple of the pad.
        for (width, bpp, stride) in [
            (600, 1, 600),
            (758, 1, 760),
            (1072, 1, 1072),
            (1236, 1, 1236),
            (1264, 1, 1264),
            (1860, 1, 1860),
            (758, 4, 3032),
            (1272, 4, 5088),
            (1860, 4, 7440),
        ] {
            assert_eq!(wire_stride(width * bpp, 4), stride, "{width} at {bpp} bpp");
            assert_eq!(wire_stride(width * bpp, 4) % 4, 0);
        }
    }

    /// `pad` of 0 answers `pixel_bytes`.
    #[test]
    fn a_pad_of_zero_is_one_byte() {
        assert_eq!(wire_stride(758, 0), 758);
    }

    /// `pack_band` writes four bytes a row for three pixels at 1 bpp, the
    /// fourth left at the 0xFF pad.
    #[test]
    fn a_short_row_keeps_its_pad() {
        #[rustfmt::skip]
        let band: Vec<u8> = vec![
            0, 0, 0,  255, 255, 255,  128, 128, 128,
            255, 0, 0,  0, 255, 0,  0, 0, 255,
        ];
        let mut wire = Vec::new();
        pack_band(&mut wire, &band, 3, 1, 4, [2, 1, 0]);
        assert_eq!(wire, [0, 255, 128, 0xFF, 76, 149, 28, 0xFF]);
    }

    /// `pack_band` scatters to `chan` at 4 bpp, leaving the fourth byte at 0xFF.
    #[test]
    fn a_wide_pixel_scatters_to_its_channels() {
        let mut wire = Vec::new();
        pack_band(&mut wire, &[10, 20, 30], 1, 4, 4, [2, 1, 0]);
        assert_eq!(wire, [30, 20, 10, 0xFF]);
    }

    /// `pack_band` answers an empty `wire` for an `xres` of 0.
    #[test]
    fn no_columns_packs_nothing() {
        let mut wire = vec![1, 2, 3];
        pack_band(&mut wire, &[], 0, 1, 4, [2, 1, 0]);
        assert!(wire.is_empty());
    }
}
