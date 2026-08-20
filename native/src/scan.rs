//! [`Book`] entries under `Config::scan_dirs`, one level deep.
//!
//! That depth excludes `Downloads/Items01/updates/`, the `.sdr` sidecar trees,
//! and any output folder written inside a scanned root.
//!
//! [`is_encrypted`] gates every entry. `engine::decrypt` copies whatever it
//! receives into `config::OUT_DIR`, and a DRM-free book yields a second copy
//! of itself.
//!
//! `Book::done` waits on every file `config::OUT_DIR` should hold for that
//! book — `convert::Targets` included — so a conversion that failed leaves the
//! book listed for another run.
//!
//! [`candidates`] is the other half: which folders the Settings panel offers,
//! read off the device rather than guessed.
//!
//! `Book::title` and `Book::cover_path` come from the filename and
//! [`THUMBNAILS_DIR`]. No book is opened.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{self, Config};
use crate::convert::Targets;
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
    /// Every output under `config::OUT_DIR` exists — `engine::output_path`
    /// and each `convert::Targets` step's.
    pub done: bool,
}

/// [`scan_in`] over `Config::scan_dirs`, `config::OUT_DIR` and
/// [`THUMBNAILS_DIR`].
pub fn scan(cfg: &Config, targets: Targets) -> Vec<Book> {
    scan_in(
        &cfg.scan_dirs,
        cfg,
        targets,
        Path::new(config::OUT_DIR),
        Path::new(THUMBNAILS_DIR),
    )
}

/// [`Book`] entries across `roots`, outputs judged against `out_dir` and
/// covers taken from `thumbs`.
pub fn scan_in(
    roots: &[PathBuf],
    cfg: &Config,
    targets: Targets,
    out_dir: &Path,
    thumbs: &Path,
) -> Vec<Book> {
    let mut out = Vec::new();
    // `roots` order, `mtime` descending within each.
    for root in roots {
        let mut found = scan_root(root, cfg, targets, out_dir, thumbs);
        found.sort_by_key(|b| std::cmp::Reverse(b.mtime));
        out.append(&mut found);
    }
    out
}

/// [`candidate`] over one directory's entries.
fn scan_root(
    root: &Path,
    cfg: &Config,
    targets: Targets,
    out_dir: &Path,
    thumbs: &Path,
) -> Vec<Book> {
    // Each root is optional on any one device.
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| candidate(&e.path(), cfg, targets, out_dir, thumbs))
        .collect()
}

/// The `Format` of a DRM'd book at `path` this `cfg` lists, or `None`.
///
/// Shared by [`candidate`] and [`count_books`], so a folder's chip counts what
/// the grid would actually show there.
fn listable(path: &Path, cfg: &Config) -> Option<Format> {
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
    if !std::fs::metadata(path).is_ok_and(|m| m.is_file()) {
        return None;
    }
    is_encrypted(path, format).then_some(format)
}

/// One [`Book`], or `None` for an entry that fails a gate.
fn candidate(
    path: &Path,
    cfg: &Config,
    targets: Targets,
    out_dir: &Path,
    thumbs: &Path,
) -> Option<Book> {
    let format = listable(path, cfg)?;
    let meta = std::fs::metadata(path).ok()?;

    // `Config::show_done` keeps or drops these.
    let done = engine::output_path(path, out_dir)
        .is_some_and(|out| out.exists() && targets.outputs(&out).iter().all(|p| p.exists()));
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

/// A folder the Settings panel offers, and what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub dir: PathBuf,
    /// DRM'd books directly inside it, `Config::show_done` aside — a folder
    /// whose books are all decrypted keeps its count and its chip.
    pub books: usize,
}

impl Candidate {
    /// The chip's label: the path relative to `config::DOCUMENTS_DIR`, or that
    /// folder's own name.
    ///
    /// A folder outside it keeps its leading `/`, which is what tells the two
    /// apart on a row of short names.
    pub fn label(&self) -> String {
        let documents = Path::new(config::DOCUMENTS_DIR);
        match self.dir.strip_prefix(documents) {
            Ok(rest) if rest.as_os_str().is_empty() => documents.file_name().map_or_else(
                || self.dir.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            ),
            Ok(rest) => rest.display().to_string(),
            Err(_) => self.dir.display().to_string(),
        }
    }
}

/// How far under `config::DOCUMENTS_DIR` [`candidates`] looks.
///
/// Two levels reaches `Downloads/Items01`, which is where current firmware
/// puts purchases, without walking a library's worth of sidecars.
const PROBE_DEPTH: usize = 2;

/// Folders that hold a DRM'd book, plus every folder already selected.
///
/// Read off the device rather than guessed: which folder a firmware downloads
/// into has moved before, and a sideload folder is whatever its owner named
/// it. A selected folder stays on the list at zero books, or deselecting it
/// would mean deselecting a chip that is no longer drawn.
pub fn candidates(cfg: &Config) -> Vec<Candidate> {
    candidates_in(Path::new(config::DOCUMENTS_DIR), cfg)
}

/// [`candidates`] over `root`.
pub fn candidates_in(root: &Path, cfg: &Config) -> Vec<Candidate> {
    let mut dirs = Vec::new();
    collect_dirs(root, PROBE_DEPTH, &mut dirs);
    // A selected folder may sit outside `root` entirely, having been written
    // into the file by hand.
    for dir in &cfg.scan_dirs {
        if !dirs.contains(dir) {
            dirs.push(dir.clone());
        }
    }

    dirs.into_iter()
        .map(|dir| Candidate {
            books: count_books(&dir, cfg),
            dir,
        })
        .filter(|c| c.books > 0 || cfg.scan_dirs.contains(&c.dir))
        .collect()
}

/// `dir`, then its subdirectories down to `depth`, in breadth order.
///
/// `.sdr` sidecars are skipped: every KFX book has one, so a library's worth of
/// them is most of what `config::DOCUMENTS_DIR` holds and none of them is a
/// folder anyone scans.
fn collect_dirs(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if !dir.is_dir() {
        return;
    }
    out.push(dir.to_path_buf());
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_none_or(|e| !e.eq_ignore_ascii_case("sdr")) && p.is_dir())
        .collect();
    children.sort();
    for child in children {
        collect_dirs(&child, depth - 1, out);
    }
}

/// [`listable`] entries directly inside `dir`.
fn count_books(dir: &Path, cfg: &Config) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| listable(&e.path(), cfg).is_some())
        .count()
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
