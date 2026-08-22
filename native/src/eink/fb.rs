//! [`Framebuffer`] — a fullscreen X11 window over a packed-RGB backing at
//! [`CH`] bytes per pixel, with [`Framebuffer::send_update`] presenting dirty
//! rows in the server's wire format. Drawing is identity; `_waveform` is ignored.

use std::path::Path;

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

/// Rec. 601 luma of `r`, `g`, `b`. The weights sum to 256.
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

pub struct Framebuffer {
    conn: RustConnection,
    win: Window,
    gc: Gcontext,
    depth: u8,
    /// Wire bytes per pixel for `depth`: 1 at depth 8, 4 at depth 24/32.
    bytes_per_pixel: usize,
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
        let xres = screen.width_in_pixels as u32;
        let yres = screen.height_in_pixels as u32;
        let depth = screen.root_depth;
        // `bits_per_pixel` for `depth`: 8 → 1 byte, 24/32 → 4 bytes.
        let bytes_per_pixel = conn
            .setup()
            .pixmap_formats
            .iter()
            .find(|f| f.depth == depth)
            .map(|f| (f.bits_per_pixel as usize / 8).max(1))
            .unwrap_or(1);
        // `[2, 1, 0]` is BGRX LSB-first, the lab126 depth-24 layout.
        let chan = wire_channels(&conn, &screen, bytes_per_pixel).unwrap_or([2, 1, 0]);
        eprintln!(
            "fb: xres={xres} yres={yres} depth={depth} bytes_per_pixel={bytes_per_pixel} \
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
                .event_mask(EventMask::EXPOSURE),
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

        // `get_geometry` against root `xres`/`yres`: a smaller or offset window
        // clips edge-anchored drawing.
        match conn
            .get_geometry(win)
            .map_err(|e| e.to_string())
            .and_then(|c| c.reply().map_err(|e| e.to_string()))
        {
            Ok(g) => {
                if u32::from(g.width) != xres || u32::from(g.height) != yres || g.x != 0 || g.y != 0
                {
                    eprintln!(
                        "fb: WARNING window geometry {}x{}+{}+{} != root {xres}x{yres}+0+0 \
                         — edge-anchored UI will be clipped",
                        g.width, g.height, g.x, g.y
                    );
                } else {
                    eprintln!("fb: window geometry matches root ({xres}x{yres})");
                }
            }
            Err(e) => eprintln!("fb: could not read window geometry: {e}"),
        }

        // `maximum_request_bytes` is the post-BIG-REQUESTS limit, ~16 MB;
        // `setup().maximum_request_length` is the ~256 KB pre-negotiation one.
        let max_req_bytes = conn.maximum_request_bytes().max(4096);
        eprintln!(
            "fb: max request {} bytes ({} rows/band at {} bpp)",
            max_req_bytes,
            max_req_bytes / (xres as usize * bytes_per_pixel).max(1),
            bytes_per_pixel
        );

        let backing = vec![0xFFu8; xres as usize * yres as usize * CH];

        Ok(Self {
            conn,
            win,
            gc,
            depth,
            bytes_per_pixel,
            chan,
            var: Var { xres, yres },
            backing,
            max_req_bytes,
        })
    }

    /// Single gray-pixel write in screen coords (0=black, 255=white), stored as
    /// `(v,v,v)`. Out-of-range silently no-ops.
    #[inline]
    pub fn put_pixel(&mut self, x: i32, y: i32, value: u8) {
        self.put_pixel_rgb(x, y, [value, value, value]);
    }

    /// Single color-pixel write in screen coords, `rgb`. Out-of-range no-ops.
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

    /// Drain the X event queue, reporting whether a repaint is due. `put_image`
    /// returns a `VoidCookie`; a request it rejects arrives here as
    /// `Event::Error`.
    pub fn pump_events(&mut self) -> bool {
        let mut needs_repaint = false;
        while let Ok(Some(event)) = self.conn.poll_for_event() {
            match event {
                Event::Expose(_) => needs_repaint = true,
                Event::Error(e) => {
                    eprintln!("x11: WARNING request failed: {e:?}");
                    needs_repaint = true;
                }
                _ => {}
            }
        }
        needs_repaint
    }

    /// Present rows `rect.top..rect.top + rect.height`, converting `backing` to
    /// the wire format per band: a luma byte at `bpp == 1`, R/G/B at `chan`
    /// offsets over a 0xFF pad otherwise. `_waveform` is ignored.
    pub fn send_update(&mut self, rect: MxcfbRect, _waveform: u32) -> Result<u32> {
        let bpp = self.bytes_per_pixel;
        let xres = self.var.xres as usize;
        let bk_stride = xres * CH; // backing bytes per scanline (RGB)
        let wire_stride = xres * bpp; // wire bytes per scanline
        let width = self.var.xres as u16;
        let top = rect.top.min(self.var.yres);
        let bottom = rect.top.saturating_add(rect.height).min(self.var.yres);
        let max_rows = (self.max_req_bytes.saturating_sub(64) / wire_stride.max(1)).max(1);
        let [rb, gb, bb] = self.chan;

        // Reused across bands; pad bytes keep the 0xFF fill.
        let mut wire: Vec<u8> = Vec::new();

        let mut y = top;
        while y < bottom {
            let h = ((bottom - y) as usize).min(max_rows);
            let s = y as usize * bk_stride;
            let e = s + h * bk_stride;
            let band = &self.backing[s..e];
            let px = h * xres;

            wire.clear();
            wire.resize(px * bpp, 0xFF);
            // `band` is `h * xres * CH` bytes; `as_chunks` leaves no remainder.
            let (triples, _) = band.as_chunks::<CH>();
            let pairs = wire.chunks_exact_mut(bpp).zip(triples);
            if bpp == 1 {
                for (w, src) in pairs {
                    w[0] = luma(src[0], src[1], src[2]);
                }
            } else {
                for (w, src) in pairs {
                    w[rb] = src[0];
                    w[gb] = src[1];
                    w[bb] = src[2];
                }
            }

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

    /// Replace `backing` with `snap`. No-op on a length mismatch.
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
        // `destroy_window` hands the screen back to the WM. Best effort.
        let _ = self.conn.destroy_window(self.win);
        let _ = self.conn.flush();
    }
}
