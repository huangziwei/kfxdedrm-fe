//! [`Config`]: which folders [`crate::scan`] reads, what it lists out of them,
//! and where [`crate::engine`] writes.
//!
//! One `key = value` file, hand-editable. [`Config::parse`] has no failure
//! mode; [`Config::sanitized`] is the one filter it applies.

use std::path::{Path, PathBuf};

/// Purchased downloads on current firmware, and the folder a fresh install
/// reads.
pub const ITEMS01_DIR: &str = "/mnt/us/documents/Downloads/Items01";
/// Library root: purchases on older models, mixed with sideloads.
/// `scan::scan_in` separates them through [`crate::mobi::is_encrypted`].
///
/// `scan::candidates` probes this and the folders under it, giving a chip to
/// each folder a firmware downloads into.
pub const DOCUMENTS_DIR: &str = "/mnt/us/documents";

/// [`Config::out_dir`] before the file names one, and where a refused value
/// reverts to.
pub const DEFAULT_OUT_DIR: &str = "/mnt/us/dedrm";

/// Why [`Config::out_dir`] cannot be `dir`, worded for `ui::keyboard`'s band.
///
/// `engine::output_path` of a MOBI under a scanned root is the book's own
/// path, and `engine::decrypt` copies it onto itself. A root one level above
/// `dir` is no match: `scan::scan_in` reads one level deep.
pub fn out_dir_refusal(dir: &Path, scan_dirs: &[PathBuf]) -> Option<&'static str> {
    if !dir.is_absolute() {
        return Some("needs a full path");
    }
    if scan_dirs.iter().any(|d| d == dir) {
        return Some("scanned for books");
    }
    None
}

/// Everything the Settings panel controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Folders `scan::scan_in` reads, each one level deep. `scan::candidates`
    /// is what offers them.
    pub scan_dirs: Vec<PathBuf>,
    /// Where `engine::decrypt` writes and `convert` puts its extra formats.
    /// `ui::keyboard` is what types it.
    pub out_dir: PathBuf,
    /// List `.kfx`, gated on the `.sdr` voucher.
    pub types_kfx: bool,
    /// List `engine::MOBI_EXTENSIONS`, gated on `mobi::is_encrypted`.
    pub types_mobi: bool,
    /// Merge the engine's `.kfx-zip` into a `.kfx` beside it. Needs the
    /// `crate::convert` add-on; without it `convert::Targets` reads it as off.
    pub pack_kfx: bool,
    /// Write a `.epub` beside it. Same add-on, same fallback.
    pub convert_epub: bool,
    /// Keep `Book::done` entries in the grid.
    pub show_done: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            scan_dirs: vec![PathBuf::from(ITEMS01_DIR)],
            out_dir: PathBuf::from(DEFAULT_OUT_DIR),
            types_kfx: true,
            types_mobi: true,
            // Off: the add-on the two of them run is not part of this install.
            pack_kfx: false,
            convert_epub: false,
            show_done: true,
        }
    }
}

/// `true`/`false` and the spellings a hand-edited file carries. `None` leaves
/// the caller's default.
fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

impl Config {
    /// [`Config::parse`] of `path`, or [`Config::default`].
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(_) => Self::default(),
        }
    }

    /// [`Config::render`] to `path`, creating its parent.
    pub fn store(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.render())
    }

    /// `key = value` lines; blank lines, `#` comments and lines without `=`
    /// are skipped. An unreadable value leaves that one field at its default.
    ///
    /// `scan_dir` carries one folder and may repeat. A file naming none takes
    /// [`Config::default`]'s; one naming it empty selects none, which
    /// [`Config::render`] writes back the same way.
    pub fn parse(text: &str) -> Self {
        let mut cfg = Self::default();
        let mut scan_dirs = Vec::new();
        let mut named_a_folder = false;

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            match key {
                "scan_dir" => {
                    named_a_folder = true;
                    if !value.is_empty() {
                        scan_dirs.push(PathBuf::from(value));
                    }
                }
                "out_dir" if !value.is_empty() => cfg.out_dir = PathBuf::from(value),
                "types_kfx" => cfg.types_kfx = parse_bool(value).unwrap_or(cfg.types_kfx),
                "types_mobi" => cfg.types_mobi = parse_bool(value).unwrap_or(cfg.types_mobi),
                "pack_kfx" => cfg.pack_kfx = parse_bool(value).unwrap_or(cfg.pack_kfx),
                "convert_epub" => cfg.convert_epub = parse_bool(value).unwrap_or(cfg.convert_epub),
                "show_done" => cfg.show_done = parse_bool(value).unwrap_or(cfg.show_done),
                _ => {}
            }
        }
        if named_a_folder {
            cfg.scan_dirs = scan_dirs;
        }
        cfg.sanitized()
    }

    /// The file format, comments included.
    pub fn render(&self) -> String {
        let mut folders = String::new();
        if self.scan_dirs.is_empty() {
            folders.push_str("scan_dir =\n");
        }
        for dir in &self.scan_dirs {
            folders.push_str(&format!("scan_dir = {}\n", dir.display()));
        }

        format!(
            "\
# kfxdedrm-fe settings. Rewritten whenever the Settings panel is used, so
# comments you add here will not survive; the values will.

# Where to look for books, one line per folder, each read one level deep.
# Settings offers a chip for every folder under {DOCUMENTS_DIR} that holds
# a DRM'd book. An empty value selects none.
{folders}
# Where decrypted books land. Created if it is missing, parents included. A
# folder listed above is refused, since the engine would copy a MOBI onto
# itself; a folder inside one is fine, at the depth the scan reads.
out_dir = {}

# Which formats to list. KFX books are listed when their .sdr voucher is
# present; MOBI-family books when their own header says they carry DRM, so
# DRM-free sideloads never appear.
types_kfx = {}
types_mobi = {}

# Extra formats, written beside the engine's own output. Both need the bokai
# add-on at {}; without it they are ignored.
#   pack_kfx      merge the .kfx-zip bundle into one .kfx container
#   convert_epub  convert the book to .epub
pack_kfx = {}
convert_epub = {}

# Keep finished books in the grid, marked with a check.
show_done = {}
",
            self.out_dir.display(),
            self.types_kfx,
            self.types_mobi,
            crate::convert::EXTENSION_DIR,
            self.pack_kfx,
            self.convert_epub,
            self.show_done,
        )
    }

    /// Every [`Config::scan_dirs`] entry absolute and listed once, an
    /// [`Config::out_dir`] [`out_dir_refusal`] accepts, and no folder in both.
    ///
    /// A relative `out_dir` takes [`DEFAULT_OUT_DIR`]. The last step drops the
    /// `scan_dir`: [`DEFAULT_OUT_DIR`] is itself a `scan_dir` a hand-edited
    /// file may name.
    fn sanitized(mut self) -> Self {
        let mut kept: Vec<PathBuf> = Vec::new();
        for dir in std::mem::take(&mut self.scan_dirs) {
            if dir.is_absolute() && !kept.contains(&dir) {
                kept.push(dir);
            }
        }
        self.scan_dirs = kept;
        if out_dir_refusal(&self.out_dir, &self.scan_dirs).is_some() {
            self.out_dir = PathBuf::from(DEFAULT_OUT_DIR);
        }
        self.scan_dirs.retain(|d| *d != self.out_dir);
        self
    }

    /// False once no folder is selected or every format is off.
    pub fn lists_anything(&self) -> bool {
        !self.scan_dirs.is_empty() && (self.types_kfx || self.types_mobi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_the_file_format() {
        let cfg = Config {
            scan_dirs: vec![
                PathBuf::from(DOCUMENTS_DIR),
                PathBuf::from("/mnt/us/documents/Sidle"),
            ],
            out_dir: PathBuf::from("/mnt/us/documents/dedrm"),
            types_kfx: true,
            types_mobi: false,
            pack_kfx: true,
            convert_epub: true,
            show_done: false,
        };
        assert_eq!(Config::parse(&cfg.render()), cfg);
        // `Config::default` is a fixed point of the same round trip.
        let d = Config::default();
        assert_eq!(Config::parse(&d.render()), d);
    }

    #[test]
    fn deselecting_every_folder_survives_the_round_trip() {
        // Distinct from a file that never mentions a folder, which takes the
        // default.
        let cfg = Config {
            scan_dirs: Vec::new(),
            ..Config::default()
        };
        assert_eq!(Config::parse(&cfg.render()), cfg);
        assert!(!cfg.lists_anything());
        assert_eq!(
            Config::parse("types_kfx = true").scan_dirs,
            [PathBuf::from(ITEMS01_DIR)]
        );
    }

    #[test]
    fn each_bad_value_costs_only_its_own_setting() {
        let cfg = Config::parse(
            "\
             # a comment\n\
             \n\
             scan_dir = /mnt/us/documents\n\
             types_kfx = maybe\n\
             types_mobi\n\
             nonsense = true\n\
             convert_epub = 1\n\
             show_done=OFF\n",
        );
        assert_eq!(cfg.scan_dirs, [PathBuf::from(DOCUMENTS_DIR)]); // read
        assert!(!cfg.show_done); // read, tolerant of spelling and spacing
        assert!(cfg.convert_epub); // read
        assert!(!cfg.pack_kfx); // absent -> default
        assert!(cfg.types_kfx); // unparseable value -> default
        assert!(cfg.types_mobi); // no `=` at all -> default
        assert_eq!(cfg.out_dir, Path::new(DEFAULT_OUT_DIR)); // absent -> default
    }

    #[test]
    fn a_folder_that_could_eat_a_book_is_refused() {
        // `engine::output_path` of a MOBI in the out folder is the book's own
        // path, and `engine::decrypt` copies it onto itself.
        let cfg = Config::parse(&format!(
            "scan_dir = {DEFAULT_OUT_DIR}\nscan_dir = relative/path"
        ));
        assert!(cfg.scan_dirs.is_empty());
        assert!(!cfg.lists_anything());
        assert_eq!(cfg.out_dir, Path::new(DEFAULT_OUT_DIR));
    }

    #[test]
    fn the_out_folder_is_never_one_of_the_scanned_folders() {
        // Both spellings of the collision: named as the out folder, and named
        // as a scanned folder.
        let typed = Config::parse(&format!(
            "scan_dir = {ITEMS01_DIR}\nout_dir = {ITEMS01_DIR}"
        ));
        assert_eq!(typed.out_dir, Path::new(DEFAULT_OUT_DIR));
        assert_eq!(typed.scan_dirs, [PathBuf::from(ITEMS01_DIR)]);

        for cfg in [typed, Config::parse("scan_dir = /mnt/us/dedrm")] {
            assert!(!cfg.scan_dirs.contains(&cfg.out_dir));
        }
    }

    #[test]
    fn an_out_folder_inside_a_scanned_folder_is_kept() {
        // `scan::scan_in` reads one level deep and never reaches it.
        let cfg = Config::parse(&format!(
            "scan_dir = {DOCUMENTS_DIR}\nout_dir = {DOCUMENTS_DIR}/dedrm"
        ));
        assert_eq!(cfg.out_dir, Path::new("/mnt/us/documents/dedrm"));
        assert_eq!(cfg.scan_dirs, [PathBuf::from(DOCUMENTS_DIR)]);
    }

    #[test]
    fn an_out_folder_that_names_no_full_path_takes_the_default() {
        for bad in ["relative/path", "dedrm", ""] {
            let cfg = Config::parse(&format!("out_dir = {bad}"));
            assert_eq!(cfg.out_dir, Path::new(DEFAULT_OUT_DIR), "{bad}");
        }
    }

    #[test]
    fn the_refusal_is_worded_for_the_keyboards_band() {
        let scanned = [PathBuf::from(ITEMS01_DIR)];
        assert_eq!(
            out_dir_refusal(Path::new("dedrm"), &scanned),
            Some("needs a full path")
        );
        assert_eq!(
            out_dir_refusal(Path::new(ITEMS01_DIR), &scanned),
            Some("scanned for books")
        );
        assert_eq!(out_dir_refusal(Path::new(DEFAULT_OUT_DIR), &scanned), None);
        // One level under a scanned root, at the depth the scan stops short of.
        assert_eq!(
            out_dir_refusal(&Path::new(ITEMS01_DIR).join("out"), &scanned),
            None
        );
    }

    #[test]
    fn a_folder_named_twice_is_scanned_once() {
        let cfg = Config::parse(&format!(
            "scan_dir = {ITEMS01_DIR}\nscan_dir = {DOCUMENTS_DIR}\nscan_dir = {ITEMS01_DIR}"
        ));
        assert_eq!(
            cfg.scan_dirs,
            [PathBuf::from(ITEMS01_DIR), PathBuf::from(DOCUMENTS_DIR)]
        );
    }

    #[test]
    fn a_fresh_install_reads_the_folder_this_firmware_downloads_into() {
        assert_eq!(Config::default().scan_dirs, [PathBuf::from(ITEMS01_DIR)]);
        assert!(Path::new(ITEMS01_DIR).starts_with(DOCUMENTS_DIR));
    }

    #[test]
    fn switching_every_format_off_is_also_an_empty_configuration() {
        let cfg = Config {
            types_kfx: false,
            types_mobi: false,
            ..Config::default()
        };
        assert!(!cfg.scan_dirs.is_empty());
        assert!(!cfg.lists_anything());
    }
}
