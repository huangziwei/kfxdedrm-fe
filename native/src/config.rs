//! [`Config`]: which folders [`crate::scan`] reads and what it lists out of
//! them.
//!
//! One `key = value` file, hand-editable. [`Config::parse`] has no failure
//! mode; [`Config::sanitized`] is the one filter it applies.
//!
//! Where the engine writes is not a setting — see [`OUT_DIR`].

use std::path::{Path, PathBuf};

/// Purchased downloads on current firmware, and the folder a fresh install
/// reads.
pub const ITEMS01_DIR: &str = "/mnt/us/documents/Downloads/Items01";
/// Library root: purchases on older models, mixed with sideloads.
/// `scan::scan_in` separates them through [`crate::mobi::is_encrypted`].
///
/// `scan::candidates` probes this and the folders under it, so wherever a
/// firmware puts its downloads there is a chip for it.
pub const DOCUMENTS_DIR: &str = "/mnt/us/documents";

/// Where the engine writes, and the one place decrypted books land.
///
/// Not a setting: the engine hardcodes this and ignores the out-folder
/// argument it is handed — see `engine`'s module header. `ui::header` is where
/// it is named on screen.
pub const OUT_DIR: &str = "/mnt/us/dedrm";

/// Everything the Settings panel controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Folders `scan::scan_in` reads, each one level deep. `scan::candidates`
    /// is what offers them.
    pub scan_dirs: Vec<PathBuf>,
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
    /// `scan_dir` carries one folder and may repeat. A file naming none at all
    /// takes [`Config::default`]'s; one naming it with an empty value has
    /// deselected every folder, which [`Config::render`] writes back the same
    /// way.
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
# Which formats to list. KFX books are listed when their .sdr voucher is
# present; MOBI-family books when their own header says they carry DRM, so
# DRM-free sideloads never appear.
types_kfx = {}
types_mobi = {}

# Extra formats, written into {OUT_DIR} beside the engine's own output. Both
# need the bokai add-on at {}; without it they are ignored.
#   pack_kfx      merge the .kfx-zip bundle into one .kfx container
#   convert_epub  convert the book to .epub
pack_kfx = {}
convert_epub = {}

# Keep finished books in the grid, marked with a check.
show_done = {}
",
            self.types_kfx,
            self.types_mobi,
            crate::convert::EXTENSION_DIR,
            self.pack_kfx,
            self.convert_epub,
            self.show_done,
        )
    }

    /// Drops any [`Config::scan_dirs`] entry that would misbehave, and any
    /// repeat of one that would not.
    ///
    /// A relative path resolves against the engine's working directory.
    /// [`OUT_DIR`] holds the engine's own output, and `engine::output_path` of
    /// a MOBI there is the file itself — the engine would copy it onto itself.
    fn sanitized(mut self) -> Self {
        let mut kept: Vec<PathBuf> = Vec::new();
        for dir in std::mem::take(&mut self.scan_dirs) {
            if dir.is_absolute() && dir != Path::new(OUT_DIR) && !kept.contains(&dir) {
                kept.push(dir);
            }
        }
        self.scan_dirs = kept;
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
        // default — so the panel's "nothing selected" is a state that keeps.
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
    }

    #[test]
    fn a_folder_that_could_eat_a_book_is_refused() {
        // `engine::output_path` of a MOBI in `OUT_DIR` is the book's own path,
        // so the engine would copy it onto itself.
        let cfg = Config::parse(&format!("scan_dir = {OUT_DIR}\nscan_dir = relative/path"));
        assert!(cfg.scan_dirs.is_empty());
        assert!(!cfg.lists_anything());
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
