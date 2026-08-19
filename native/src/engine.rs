//! [`locate`] the kfxdedrm engine under [`BIN_DIR`], [`Engine::decrypt`] to
//! run it. Nothing in this crate decrypts anything.
//!
//! The engine's command surface:
//!
//! | invocation | effect |
//! |:--|:--|
//! | *(no args)* | decrypt everything under `documents/` |
//! | `test` | exits 0 if this build runs on this device |
//! | `dedrm <book> [outdir]` | decrypt one book |
//! | `dedrm_all [scandir] [outdir]` | decrypt a directory |
//! | `keyfile [scandir]` | write a desktop-plugin keyfile |
//! | `scan` / `scantruncate [dir] [menu]` | rewrite kfxdedrm's own menu |
//!
//! [`probe_in`] calls `test` and [`Engine::decrypt`] calls `dedrm`.
//! `scan`/`scantruncate` write into [`EXTENSION_DIR`]. `dedrm_all` carries no
//! per-book progress.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The engine extension's root, distinct from this app's `kfxdedrm-fe`.
pub const EXTENSION_DIR: &str = "/mnt/us/extensions/kfxdedrm";
/// Where the engine's four ABI builds live.
pub const BIN_DIR: &str = "/mnt/us/extensions/kfxdedrm/bin";

/// Shown verbatim by [`crate::ui::setup`].
pub const RELEASES_URL: &str = "github.com/Satsuoni/DeDRM_tools/releases";
/// The MOBI-capable asset. `kfxdedrm_kual.zip` covers KFX alone.
pub const RELEASE_ASSET: &str = "kfxdedrmmobi.zip";

/// The engine's four builds in [`probe_in`] order: hard-float first,
/// soft-float second, `_c11` ahead of `_old` within each.
pub const ABI_VARIANTS: [&str; 4] = [
    "kfxdedrmhf_c11",
    "kfxdedrmhf_old",
    "kfxdedrm_old",
    "kfxdedrm_c11",
];

/// Why [`locate`] found nothing. [`crate::ui::setup`] words the two apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    /// No [`BIN_DIR`].
    NotInstalled,
    /// [`BIN_DIR`] holds no build whose `test` exits 0.
    NoWorkingBuild,
}

/// The build [`locate`] resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Engine {
    exe: PathBuf,
}

/// The engine's two code paths, which [`output_path`] names differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// KFX container, keyed by a `.sdr` voucher sidecar.
    Kfx,
    /// [`MOBI_EXTENSIONS`].
    Mobi,
}

/// The extensions the engine names as MOBI book candidates.
pub const MOBI_EXTENSIONS: [&str; 3] = ["azw3", "azw4", "mobi"];

impl Format {
    /// By extension, case-insensitively: a FAT partition carries `.AZW3`.
    pub fn of_path(path: &Path) -> Option<Format> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        if ext == "kfx" {
            Some(Format::Kfx)
        } else if MOBI_EXTENSIONS.contains(&ext.as_str()) {
            Some(Format::Mobi)
        } else {
            None
        }
    }
}

/// The engine's output for `book` under `out_dir`.
///
/// [`Format::Kfx`] takes the `.kfx-zip` extension. [`Format::Mobi`] keeps its
/// own filename, copied into `out_dir` and patched in place.
pub fn output_path(book: &Path, out_dir: &Path) -> Option<PathBuf> {
    match Format::of_path(book)? {
        Format::Kfx => {
            let stem = book.file_stem()?.to_str()?;
            Some(out_dir.join(format!("{stem}.kfx-zip")))
        }
        Format::Mobi => Some(out_dir.join(book.file_name()?)),
    }
}

/// [`ABI_VARIANTS`] under `dir`, in probe order.
pub fn variant_paths(dir: &Path) -> Vec<PathBuf> {
    ABI_VARIANTS.iter().map(|n| dir.join(n)).collect()
}

/// [`locate_in`] over [`BIN_DIR`].
pub fn locate() -> Result<Engine, Missing> {
    locate_in(Path::new(BIN_DIR))
}

/// The [`Engine`] under `dir`, or why there is none.
pub fn locate_in(dir: &Path) -> Result<Engine, Missing> {
    if !dir.is_dir() {
        return Err(Missing::NotInstalled);
    }
    probe_in(dir)
        .map(|exe| Engine { exe })
        .ok_or(Missing::NoWorkingBuild)
}

/// The first [`variant_paths`] entry whose `test` exits 0.
///
/// Each variant targets a different ABI; three of the four fail to start on
/// any one device.
fn probe_in(dir: &Path) -> Option<PathBuf> {
    variant_paths(dir).into_iter().find(|exe| {
        exe.is_file()
            && Command::new(exe)
                .arg("test")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
    })
}

impl Engine {
    /// The path [`Engine::decrypt`] spawns.
    pub fn exe(&self) -> &Path {
        &self.exe
    }

    /// `<exe> dedrm <book> <out_dir>`.
    ///
    /// `out_dir` rides every call, matching `Config::out_dir` against what the
    /// engine receives. The engine creates the folder.
    pub fn decrypt(&self, book: &Path, out_dir: &Path) -> Command {
        let mut cmd = Command::new(&self.exe);
        cmd.arg("dedrm").arg(book).arg(out_dir);
        cmd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_order_matches_the_engines_own_launcher() {
        let paths = variant_paths(Path::new("/x/bin"));
        let names: Vec<&str> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        // Hard-float first: on a device that runs both, this is the one the
        // engine's run_cmd.sh would have picked.
        assert_eq!(
            names,
            [
                "kfxdedrmhf_c11",
                "kfxdedrmhf_old",
                "kfxdedrm_old",
                "kfxdedrm_c11"
            ]
        );
    }

    #[test]
    fn kfx_output_takes_a_new_extension_and_mobi_keeps_its_own() {
        let out = Path::new("/mnt/us/dedrm");
        assert_eq!(
            output_path(Path::new("/d/Items01/Book_B000O76ON6.kfx"), out),
            Some(out.join("Book_B000O76ON6.kfx-zip"))
        );
        // The MOBI path copies the file under its own name — same name, same
        // extension, different directory.
        assert_eq!(
            output_path(Path::new("/d/Some Book.azw3"), out),
            Some(out.join("Some Book.azw3"))
        );
    }

    #[test]
    fn a_dotted_title_keeps_everything_but_the_real_extension() {
        // `file_stem` splits at the last dot, so the volume number survives.
        assert_eq!(
            output_path(
                Path::new("/d/All of Us_ Vol. 1_B00XST7S8C.kfx"),
                Path::new("/o")
            ),
            Some(PathBuf::from("/o/All of Us_ Vol. 1_B00XST7S8C.kfx-zip"))
        );
    }

    #[test]
    fn classifies_only_what_the_engine_has_a_path_for() {
        assert_eq!(Format::of_path(Path::new("a.kfx")), Some(Format::Kfx));
        for ext in MOBI_EXTENSIONS {
            assert_eq!(
                Format::of_path(&PathBuf::from(format!("a.{ext}"))),
                Some(Format::Mobi),
                "{ext}"
            );
        }
        // FAT round-trips through desktops that upper-case extensions.
        assert_eq!(Format::of_path(Path::new("a.AZW3")), Some(Format::Mobi));
        assert_eq!(Format::of_path(Path::new("a.KFX")), Some(Format::Kfx));
        // The engine names no candidate for either of these.
        assert_eq!(Format::of_path(Path::new("a.azw")), None);
        assert_eq!(Format::of_path(Path::new("a.prc")), None);
        // Everything else, including the engine's own output.
        assert_eq!(Format::of_path(Path::new("a.kfx-zip")), None);
        assert_eq!(Format::of_path(Path::new("a.epub")), None);
        assert_eq!(Format::of_path(Path::new("noext")), None);
        assert_eq!(output_path(Path::new("noext"), Path::new("/o")), None);
    }
}
