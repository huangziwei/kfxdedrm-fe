//! [`Book`] entries under `Config::scan_roots`, one level deep.
//!
//! That depth excludes `Downloads/Items01/updates/`, the `.sdr` sidecar trees,
//! and any output folder written inside a scanned root.
//!
//! [`is_encrypted`] gates every entry. `engine::decrypt` copies whatever it
//! receives into `Config::out_dir`, and a DRM-free book yields a second copy
//! of itself.
//!
//! `Book::title` and `Book::cover_path` come from the filename and
//! [`THUMBNAILS_DIR`]. No book is opened.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::engine::{self, Format};
use crate::mobi;

/// Cover-thumbnail cache, keyed by `Book::asin`.
pub const THUMBNAILS_DIR: &str = "/mnt/us/system/thumbnails";

/// One entry in the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Book {
    /// The encrypted file, `engine::decrypt`'s input.
    pub path: PathBuf,
    /// Selects `engine::output_path`'s naming.
    pub format: Format,
    /// Display title, from [`title_from_stem`].
    pub title: String,
    /// The trailing `_<ASIN>` token, from [`parse_asin`].
    pub asin: Option<String>,
    /// A complete thumbnail under [`THUMBNAILS_DIR`].
    pub cover_path: Option<PathBuf>,
    /// Size of `path`.
    pub size: u64,
    /// Sort key within a root, newest first.
    pub mtime: SystemTime,
    /// `engine::output_path` exists under `Config::out_dir`.
    pub done: bool,
}

/// [`scan_in`] over `Config::scan_roots` and [`THUMBNAILS_DIR`].
pub fn scan(cfg: &Config) -> Vec<Book> {
    scan_in(&cfg.scan_roots(), cfg, Path::new(THUMBNAILS_DIR))
}

/// [`Book`] entries across `roots`, covers taken from `thumbs`.
pub fn scan_in(roots: &[PathBuf], cfg: &Config, thumbs: &Path) -> Vec<Book> {
    let mut out = Vec::new();
    // `roots` order, `mtime` descending within each.
    for root in roots {
        let mut found = scan_root(root, cfg, thumbs);
        found.sort_by_key(|b| std::cmp::Reverse(b.mtime));
        out.append(&mut found);
    }
    out
}

/// [`candidate`] over one directory's entries.
fn scan_root(root: &Path, cfg: &Config, thumbs: &Path) -> Vec<Book> {
    // Each root is optional on any one device.
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| candidate(&e.path(), cfg, thumbs))
        .collect()
}

/// One [`Book`], or `None` for an entry that fails a gate.
fn candidate(path: &Path, cfg: &Config, thumbs: &Path) -> Option<Book> {
    let name = path.file_name()?.to_str()?;
    // AppleDouble shadows carry the name of a real file on a FAT partition.
    if name.starts_with("._") {
        return None;
    }

    let format = Format::of_path(path)?;
    let wanted = match format {
        Format::Kfx => cfg.types_kfx,
        Format::Mobi => cfg.types_mobi,
    };
    if !wanted {
        return None;
    }

    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }

    if !is_encrypted(path, format) {
        return None;
    }

    // `Config::show_done` keeps or drops these.
    let done = engine::output_path(path, &cfg.out_dir).is_some_and(|p| p.exists());
    if done && !cfg.show_done {
        return None;
    }

    let stem = path.file_stem()?.to_str()?;
    let asin = parse_asin(stem);
    Some(Book {
        title: title_from_stem(stem, asin.as_deref()),
        cover_path: asin.as_deref().and_then(|a| cover_for(thumbs, a)),
        asin,
        format,
        size: meta.len(),
        mtime: meta.modified().unwrap_or(UNIX_EPOCH),
        done,
        path: path.to_path_buf(),
    })
}

/// Whether `path` carries DRM.
///
/// [`Format::Kfx`] takes [`voucher_path`], which is the decrypt key and the
/// mark of a finished download. [`Format::Mobi`] takes its own header.
fn is_encrypted(path: &Path, format: Format) -> bool {
    match format {
        Format::Kfx => voucher_path(path).is_some_and(|v| v.is_file()),
        Format::Mobi => mobi::is_encrypted(path),
    }
}

/// `<stem>.sdr/assets/voucher` beside `kfx`.
fn voucher_path(kfx: &Path) -> Option<PathBuf> {
    let stem = kfx.file_stem()?.to_str()?;
    Some(
        kfx.parent()?
            .join(format!("{stem}.sdr"))
            .join("assets/voucher"),
    )
}

/// The final `_`-delimited token of `stem`, when it is `B` plus nine
/// uppercase alphanumerics.
fn parse_asin(stem: &str) -> Option<String> {
    let tok = stem.rsplit('_').next()?;
    let well_formed = tok.len() == 10
        && tok.starts_with('B')
        && tok
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
    well_formed.then(|| tok.to_string())
}

/// `stem` without its trailing `_<asin>`, `_ ` restored to `: `.
fn title_from_stem(stem: &str, asin: Option<&str>) -> String {
    let title = asin
        .and_then(|a| stem.strip_suffix(&format!("_{a}")))
        .unwrap_or(stem);
    title.replace("_ ", ": ")
}

/// The complete thumbnail for `asin` under `thumbs`. A `.tmp.partial` name
/// marks a half-written JPEG and reads as absent.
fn cover_for(thumbs: &Path, asin: &str) -> Option<PathBuf> {
    let p = thumbs.join(format!("thumbnail_{asin}_EBOK_portrait.jpg"));
    p.is_file().then_some(p)
}
