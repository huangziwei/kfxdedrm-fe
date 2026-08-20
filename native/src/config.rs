//! [`Config`]: what [`crate::scan`] reads and where `engine::decrypt` writes.
//!
//! One `key = value` file, hand-editable. [`Config::parse`] has no failure
//! mode; [`Config::sanitized`] is the one filter it applies.

use std::path::{Path, PathBuf};

/// Purchased downloads on current firmware.
pub const ITEMS01_DIR: &str = "/mnt/us/documents/Downloads/Items01";
/// Library root: purchases on older models, mixed with sideloads.
/// `scan::scan_in` separates them through [`crate::mobi::is_encrypted`].
pub const DOCUMENTS_DIR: &str = "/mnt/us/documents";
/// `engine::decrypt` writes here given no out-folder argument.
pub const DEFAULT_OUT_DIR: &str = "/mnt/us/dedrm";

/// Stops for `ui::configmenu::OutDirs`.
///
/// The second is a subdirectory of [`DOCUMENTS_DIR`]; `scan::scan_in` reads
/// one level deep and does not reach it. Any other path reaches `out_dir`
/// through the file, and `OutDirs::new` keeps it as a stop.
pub const OUT_DIR_PRESETS: [&str; 2] = [DEFAULT_OUT_DIR, "/mnt/us/documents/dedrm"];

/// Everything the Settings panel controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Scan [`ITEMS01_DIR`].
    pub scan_items01: bool,
    /// Scan [`DOCUMENTS_DIR`].
    pub scan_documents: bool,
    /// List `.kfx`, gated on the `.sdr` voucher.
    pub types_kfx: bool,
    /// List `engine::MOBI_EXTENSIONS`, gated on `mobi::is_encrypted`.
    pub types_mobi: bool,
    /// `engine::decrypt`'s out-folder argument.
    pub out_dir: PathBuf,
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
            scan_items01: true,
            scan_documents: true,
            types_kfx: true,
            types_mobi: true,
            out_dir: PathBuf::from(DEFAULT_OUT_DIR),
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
    pub fn parse(text: &str) -> Self {
        let mut cfg = Self::default();
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
                "scan_items01" => cfg.scan_items01 = parse_bool(value).unwrap_or(cfg.scan_items01),
                "scan_documents" => {
                    cfg.scan_documents = parse_bool(value).unwrap_or(cfg.scan_documents)
                }
                "types_kfx" => cfg.types_kfx = parse_bool(value).unwrap_or(cfg.types_kfx),
                "types_mobi" => cfg.types_mobi = parse_bool(value).unwrap_or(cfg.types_mobi),
                "pack_kfx" => cfg.pack_kfx = parse_bool(value).unwrap_or(cfg.pack_kfx),
                "convert_epub" => cfg.convert_epub = parse_bool(value).unwrap_or(cfg.convert_epub),
                "show_done" => cfg.show_done = parse_bool(value).unwrap_or(cfg.show_done),
                "out_dir" if !value.is_empty() => cfg.out_dir = PathBuf::from(value),
                _ => {}
            }
        }
        cfg.sanitized()
    }

    /// The file format, comments included.
    pub fn render(&self) -> String {
        format!(
            "\
# kfxdedrm-fe settings. Rewritten whenever the Settings panel is used, so
# comments you add here will not survive; the values will.

# Where to look for books. Each directory is read one level deep.
#   items01    purchases on current firmware
#   documents  purchases on older models, and your own sideloads
scan_items01 = {}
scan_documents = {}

# Which formats to list. KFX books are listed when their .sdr voucher is
# present; MOBI-family books when their own header says they carry DRM, so
# DRM-free sideloads never appear.
types_kfx = {}
types_mobi = {}

# Where decrypted books are written. Passed to the engine as its out folder.
out_dir = {}

# Extra formats, written into out_dir beside the engine's own output. Both
# need the bokai add-on at {}; without it they are ignored.
#   pack_kfx      merge the .kfx-zip bundle into one .kfx container
#   convert_epub  convert the book to .epub
pack_kfx = {}
convert_epub = {}

# Keep finished books in the grid, marked with a check.
show_done = {}
",
            self.scan_items01,
            self.scan_documents,
            self.types_kfx,
            self.types_mobi,
            self.out_dir.display(),
            crate::convert::EXTENSION_DIR,
            self.pack_kfx,
            self.convert_epub,
            self.show_done,
        )
    }

    /// Resets `out_dir` to [`DEFAULT_OUT_DIR`] for two values.
    ///
    /// A relative path resolves against the engine's working directory. A scan
    /// root receives `engine::output_path` of a MOBI at its own path, and the
    /// engine copies that file onto itself.
    fn sanitized(mut self) -> Self {
        let bad = !self.out_dir.is_absolute()
            || [ITEMS01_DIR, DOCUMENTS_DIR]
                .iter()
                .any(|d| self.out_dir == Path::new(d));
        if bad {
            self.out_dir = PathBuf::from(DEFAULT_OUT_DIR);
        }
        self
    }

    /// Roots for `scan::scan_in`, in listing order.
    pub fn scan_roots(&self) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        if self.scan_items01 {
            roots.push(PathBuf::from(ITEMS01_DIR));
        }
        if self.scan_documents {
            roots.push(PathBuf::from(DOCUMENTS_DIR));
        }
        roots
    }

    /// False once every root or every format is off.
    pub fn lists_anything(&self) -> bool {
        (self.scan_items01 || self.scan_documents) && (self.types_kfx || self.types_mobi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_the_file_format() {
        let cfg = Config {
            scan_items01: false,
            scan_documents: true,
            types_kfx: true,
            types_mobi: false,
            out_dir: PathBuf::from("/mnt/us/documents/dedrm"),
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
    fn each_bad_value_costs_only_its_own_setting() {
        let cfg = Config::parse(
            "\
             # a comment\n\
             \n\
             scan_items01 = no\n\
             types_kfx = maybe\n\
             types_mobi\n\
             nonsense = true\n\
             convert_epub = 1\n\
             show_done=OFF\n",
        );
        assert!(!cfg.scan_items01); // read
        assert!(!cfg.show_done); // read, tolerant of spelling and spacing
        assert!(cfg.convert_epub); // read
        assert!(!cfg.pack_kfx); // absent -> default
        assert!(cfg.types_kfx); // unparseable value -> default
        assert!(cfg.types_mobi); // no `=` at all -> default
        assert_eq!(cfg.out_dir, Path::new(DEFAULT_OUT_DIR)); // absent -> default
    }

    #[test]
    fn an_output_folder_that_could_eat_a_book_is_refused() {
        // `engine::output_path` of a MOBI under a scanned root is the book's
        // own path.
        for bad in [DOCUMENTS_DIR, ITEMS01_DIR, "relative/path", ""] {
            let cfg = Config::parse(&format!("out_dir = {bad}"));
            assert_eq!(
                cfg.out_dir,
                Path::new(DEFAULT_OUT_DIR),
                "{bad} should have been refused"
            );
        }
        // A subdirectory of a scanned root is fine — the scan is one level deep,
        // so nothing written there is ever read back as an input.
        let cfg = Config::parse("out_dir = /mnt/us/documents/dedrm");
        assert_eq!(cfg.out_dir, Path::new("/mnt/us/documents/dedrm"));
    }

    #[test]
    fn scan_roots_follow_the_toggles_in_listing_order() {
        let mut cfg = Config::default();
        assert_eq!(
            cfg.scan_roots(),
            vec![PathBuf::from(ITEMS01_DIR), PathBuf::from(DOCUMENTS_DIR)]
        );
        cfg.scan_items01 = false;
        assert_eq!(cfg.scan_roots(), vec![PathBuf::from(DOCUMENTS_DIR)]);
        cfg.scan_documents = false;
        assert!(cfg.scan_roots().is_empty());
        assert!(!cfg.lists_anything());
    }

    #[test]
    fn switching_every_format_off_is_also_an_empty_configuration() {
        let cfg = Config {
            types_kfx: false,
            types_mobi: false,
            ..Config::default()
        };
        assert!(!cfg.scan_roots().is_empty());
        assert!(!cfg.lists_anything());
    }
}
