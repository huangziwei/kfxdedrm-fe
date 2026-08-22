//! [`locate`] the bokai converter under [`BIN_DIR`], [`Targets`] for what the
//! settings ask of it, [`Converter::convert`] to run one [`Step`].
//!
//! bokai is an add-on, not a dependency: [`locate`] returning `None` leaves
//! the app decrypting and doing nothing else. `crate::install` fetches it from
//! the release named by [`RELEASES_URL`], into an extension of its own beside
//! `crate::engine`'s under `/mnt/us/extensions/`.
//!
//! The converter's command surface:
//!
//! | invocation | effect |
//! |:--|:--|
//! | `--version` | exits 0 if this build runs on this device |
//! | `convert <in> <out>` | both formats read off the two extensions |
//!
//! Every conversion here starts from `engine::output_path` — `<stem>.kfx-zip`
//! for a KFX book, the book's own name for a MOBI-family one — and writes
//! beside it, inside `config::OUT_DIR`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::Config;

/// The add-on extension's root, distinct from this app's and the engine's.
pub const EXTENSION_DIR: &str = "/mnt/us/extensions/bokai";
/// Where the zip installs bokai's ABI builds.
pub const BIN_DIR: &str = "/mnt/us/extensions/bokai/bin";

/// Where `install::SOURCES` fetches it from, and what the log names when that
/// fails and it has to be done by hand instead.
pub const RELEASES_URL: &str = "github.com/huangziwei/sidle/releases";
/// The asset, `*` standing for the version. bokai versions on its own line
/// and moves without this app moving, so no one version belongs here — which
/// is why `install` matches it by pattern rather than by name.
pub const RELEASE_ASSET: &str = "bokai-*-kindle.zip";

/// bokai's two builds in [`locate_in`] order: hard-float first, soft-float
/// second. One zip carries both and a device starts one of them.
pub const ABI_VARIANTS: [&str; 2] = ["bokai", "bokai-armsf"];

/// Extensions bokai reads.
///
/// `azw4` is an `engine::MOBI_EXTENSIONS` entry bokai's own format detection
/// does not name; a step over one would only fail.
const READABLE: [&str; 4] = ["kfx-zip", "kfx", "azw3", "mobi"];

/// The binary [`locate`] resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converter {
    exe: PathBuf,
}

/// [`ABI_VARIANTS`] under `dir`, in probe order.
pub fn variant_paths(dir: &Path) -> Vec<PathBuf> {
    ABI_VARIANTS.iter().map(|n| dir.join(n)).collect()
}

/// [`locate_in`] over [`BIN_DIR`].
pub fn locate() -> Option<Converter> {
    locate_in(Path::new(BIN_DIR))
}

/// The first [`variant_paths`] entry under `dir` that [`locate_at`] accepts.
///
/// Each variant targets a different float ABI, so at most one of them starts
/// on any one device.
pub fn locate_in(dir: &Path) -> Option<Converter> {
    variant_paths(dir)
        .into_iter()
        .find_map(|exe| locate_at(&exe))
}

/// `exe`, if it is a file whose `--version` exits 0.
///
/// The run costs one process and rules out a build for the wrong ABI, which
/// otherwise fails once per book with the panel mid-decrypt.
pub fn locate_at(exe: &Path) -> Option<Converter> {
    let ok = exe.is_file()
        && Command::new(exe)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    ok.then(|| Converter {
        exe: exe.to_path_buf(),
    })
}

/// The extra format a [`Step`] produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The engine's `.kfx-zip` bundle merged into one `.kfx` container.
    Kfx,
    /// EPUB, through bokai's own intermediate representation.
    Epub,
}

impl Kind {
    /// The extension [`Step::output`] takes.
    pub fn extension(self) -> &'static str {
        match self {
            Kind::Kfx => "kfx",
            Kind::Epub => "epub",
        }
    }

    /// Banner line while the step runs.
    pub fn progress(self) -> &'static str {
        match self {
            Kind::Kfx => "Packing as KFX…",
            Kind::Epub => "Converting to EPUB…",
        }
    }

    /// Name in a result banner.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Kfx => "KFX",
            Kind::Epub => "EPUB",
        }
    }
}

/// One `convert` run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub kind: Kind,
    /// An earlier step's [`Step::output`], or the engine's own output.
    pub input: PathBuf,
    pub output: PathBuf,
}

/// The two switches, resolved against what is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Targets {
    pub kfx: bool,
    pub epub: bool,
}

impl Targets {
    /// `Config::pack_kfx` and `Config::convert_epub`, both off without a
    /// [`Converter`].
    ///
    /// A switch left on in the file names a binary that is not there, and
    /// `scan::Book::done` would then never come true for any book.
    pub fn new(cfg: &Config, converter: Option<&Converter>) -> Self {
        match converter {
            Some(_) => Targets {
                kfx: cfg.pack_kfx,
                epub: cfg.convert_epub,
            },
            None => Targets::default(),
        }
    }

    pub fn any(self) -> bool {
        self.kfx || self.epub
    }

    /// The conversions for `decrypted`, in run order.
    ///
    /// `decrypted` is `engine::output_path`'s result. [`Kind::Kfx`] applies to
    /// a `.kfx-zip` alone: a MOBI-family book is copied under its own name and
    /// has no bundle to merge.
    pub fn steps(self, decrypted: &Path) -> Vec<Step> {
        let mut steps = Vec::new();
        if !self.any() || !readable(decrypted) {
            return steps;
        }

        if self.kfx && is_kfx_zip(decrypted) {
            steps.push(Step {
                kind: Kind::Kfx,
                input: decrypted.to_path_buf(),
                output: decrypted.with_extension(Kind::Kfx.extension()),
            });
        }
        if self.epub {
            // From the packed KFX when this run produces one: the merge is the
            // cheaper half of the two, and the EPUB then comes off a single
            // container rather than the bundle a second time.
            let input = steps
                .first()
                .map_or_else(|| decrypted.to_path_buf(), |s| s.output.clone());
            steps.push(Step {
                kind: Kind::Epub,
                input,
                output: decrypted.with_extension(Kind::Epub.extension()),
            });
        }
        steps
    }

    /// What [`Targets::steps`] writes. `scan::Book::done` waits on all of it.
    pub fn outputs(self, decrypted: &Path) -> Vec<PathBuf> {
        self.steps(decrypted)
            .into_iter()
            .map(|s| s.output)
            .collect()
    }
}

/// The engine's KFX output, whose extension no other format shares.
fn is_kfx_zip(path: &Path) -> bool {
    extension_is(path, "kfx-zip")
}

/// Whether bokai has an importer for `path`'s extension.
fn readable(path: &Path) -> bool {
    READABLE.iter().any(|ext| extension_is(path, ext))
}

/// `path`'s extension equals `ext`, case-insensitively: a FAT partition
/// carries `.AZW3`.
fn extension_is(path: &Path, ext: &str) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

impl Converter {
    /// The path [`Converter::convert`] spawns.
    pub fn exe(&self) -> &Path {
        &self.exe
    }

    /// `<exe> convert <input> <output>`.
    ///
    /// bokai takes both formats off the extensions, so neither `-f` nor `-t`
    /// rides the call. Its progress goes to stderr, which `launch.sh` has
    /// already pointed at the log.
    pub fn convert(&self, step: &Step) -> Command {
        let mut cmd = Command::new(&self.exe);
        cmd.arg("convert").arg(&step.input).arg(&step.output);
        cmd
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    const BOTH: Targets = Targets {
        kfx: true,
        epub: true,
    };

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kfxdedrm-fe-convert-{name}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// A build under `dir` whose `--version` exits `code`, standing in for one
    /// the loader accepts (`0`) or refuses (anything else).
    fn variant(dir: &Path, name: &str, code: i32) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\nexit {code}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn kinds(steps: &[Step]) -> Vec<Kind> {
        steps.iter().map(|s| s.kind).collect()
    }

    #[test]
    fn nothing_is_planned_without_a_converter() {
        let cfg = Config {
            pack_kfx: true,
            convert_epub: true,
            ..Config::default()
        };
        let targets = Targets::new(&cfg, None);
        assert_eq!(targets, Targets::default());
        assert!(!targets.any());
        assert!(targets.steps(Path::new("/o/Book.kfx-zip")).is_empty());
    }

    #[test]
    fn the_epub_comes_off_the_kfx_this_run_packs() {
        let steps = BOTH.steps(Path::new("/o/Book.kfx-zip"));
        assert_eq!(kinds(&steps), [Kind::Kfx, Kind::Epub]);
        assert_eq!(steps[0].input, Path::new("/o/Book.kfx-zip"));
        assert_eq!(steps[0].output, Path::new("/o/Book.kfx"));
        // The second step reads the first one's output, not the bundle.
        assert_eq!(steps[1].input, steps[0].output);
        assert_eq!(steps[1].output, Path::new("/o/Book.epub"));
    }

    #[test]
    fn an_epub_alone_is_read_straight_out_of_the_bundle() {
        let targets = Targets {
            kfx: false,
            epub: true,
        };
        let steps = targets.steps(Path::new("/o/Book.kfx-zip"));
        assert_eq!(kinds(&steps), [Kind::Epub]);
        assert_eq!(steps[0].input, Path::new("/o/Book.kfx-zip"));
        assert_eq!(steps[0].output, Path::new("/o/Book.epub"));
    }

    #[test]
    fn a_mobi_family_copy_has_no_bundle_to_pack() {
        // The engine copies these under their own name; `Kind::Kfx` has no
        // input here even with the switch on.
        for name in ["Some Book.azw3", "Some Book.mobi"] {
            let steps = BOTH.steps(&PathBuf::from("/o").join(name));
            assert_eq!(kinds(&steps), [Kind::Epub], "{name}");
            assert_eq!(steps[0].output, Path::new("/o/Some Book.epub"), "{name}");
        }
    }

    #[test]
    fn a_format_bokai_cannot_read_is_left_alone() {
        // `engine::MOBI_EXTENSIONS` carries azw4; bokai names no importer for
        // it, so a step would only spend a process on an error.
        assert!(BOTH.steps(Path::new("/o/Some Book.azw4")).is_empty());
        assert!(BOTH.steps(Path::new("/o/noext")).is_empty());
    }

    #[test]
    fn a_dotted_title_keeps_everything_but_the_real_extension() {
        let steps = BOTH.steps(Path::new("/o/All of Us_ Vol. 1_B00XST7S8C.kfx-zip"));
        assert_eq!(
            steps[0].output,
            Path::new("/o/All of Us_ Vol. 1_B00XST7S8C.kfx")
        );
        assert_eq!(
            steps[1].output,
            Path::new("/o/All of Us_ Vol. 1_B00XST7S8C.epub")
        );
    }

    #[test]
    fn an_upper_cased_extension_is_the_same_extension() {
        // FAT round-trips through desktops that upper-case extensions.
        let steps = BOTH.steps(Path::new("/o/Some Book.AZW3"));
        assert_eq!(kinds(&steps), [Kind::Epub]);
    }

    #[test]
    fn the_outputs_are_the_steps_own_outputs() {
        let decrypted = Path::new("/o/Book.kfx-zip");
        assert_eq!(
            BOTH.outputs(decrypted),
            vec![PathBuf::from("/o/Book.kfx"), PathBuf::from("/o/Book.epub")]
        );
        assert!(Targets::default().outputs(decrypted).is_empty());
    }

    #[test]
    fn a_missing_binary_resolves_to_no_converter() {
        assert_eq!(locate_at(Path::new("/nonexistent/bokai")), None);
        assert_eq!(locate_in(Path::new("/nonexistent/bin")), None);
    }

    #[test]
    fn probe_order_puts_the_hard_float_build_first() {
        let paths = variant_paths(Path::new("/x/bin"));
        let names: Vec<&str> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        // On a device that starts both, this is the one to run.
        assert_eq!(names, ["bokai", "bokai-armsf"]);
    }

    #[test]
    fn a_soft_float_only_install_still_resolves() {
        // What is left after unpacking a zip whose hard-float build a device
        // cannot start: the name `locate` used to look for is not there.
        let dir = tmpdir("armsf-only");
        let armsf = variant(&dir, "bokai-armsf", 0);
        assert_eq!(locate_in(&dir).map(|c| c.exe().to_path_buf()), Some(armsf));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_build_that_will_not_start_is_passed_over_for_one_that_will() {
        // Both installed, which is every install: the probe, not the name,
        // decides.
        let dir = tmpdir("both");
        variant(&dir, "bokai", 126);
        let armsf = variant(&dir, "bokai-armsf", 0);
        assert_eq!(locate_in(&dir).map(|c| c.exe().to_path_buf()), Some(armsf));

        // And with the hard-float build running, it wins on order.
        let hf = variant(&dir, "bokai", 0);
        assert_eq!(locate_in(&dir).map(|c| c.exe().to_path_buf()), Some(hf));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_where_nothing_starts_resolves_to_no_converter() {
        let dir = tmpdir("none");
        variant(&dir, "bokai", 1);
        variant(&dir, "bokai-armsf", 1);
        assert_eq!(locate_in(&dir), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_kind_names_itself_in_three_places() {
        for kind in [Kind::Kfx, Kind::Epub] {
            assert!(!kind.extension().is_empty());
            assert!(!kind.progress().is_empty());
            assert!(!kind.label().is_empty());
        }
        assert_ne!(Kind::Kfx.extension(), Kind::Epub.extension());
        assert_ne!(Kind::Kfx.progress(), Kind::Epub.progress());
    }
}
