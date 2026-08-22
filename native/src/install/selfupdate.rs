//! [`update`] fetches the newest kfxdedrm-fe release into [`STAGING_DIR`].
//! `bin/launch.sh` moves it onto [`EXTENSION_DIR`] at the next start, over
//! [`SHARED_FILES`] carried from the copy it replaces.

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;

use super::{Release, Source, between, download_and_unpack, http};
use crate::log;

/// Where this app is installed.
pub const EXTENSION_DIR: &str = "/mnt/us/extensions/kfxdedrm-fe";
/// What `bin/launch.sh` moves into [`EXTENSION_DIR`] on the next start.
pub const STAGING_DIR: &str = "/mnt/us/extensions/kfxdedrm-fe.new";

/// Where [`update`] fetches from, and the page the log names on a failure.
pub const RELEASES_URL: &str = "github.com/huangziwei/kfxdedrm-fe/releases";
/// The asset, `*` standing for the tag the filename carries.
pub const RELEASE_ASSET: &str = "kfxdedrm-fe-*-kindle.zip";

/// The two builds in [`runnable`] order, hard-float first. The `VARIANTS` line
/// of `bin/launch.sh` names the same two.
pub const ABI_VARIANTS: [&str; 2] = ["kfxdedrm-fe", "kfxdedrm-fe-armsf"];

/// The invocation that draws nothing and exits 0. `main.rs` answers it,
/// `bin/launch.sh` picks a build with it, and [`runnable`] proves a download
/// with it.
pub const VERSION_FLAG: &str = "--version";

/// The files both frontends write inside [`EXTENSION_DIR`]. The release
/// archive ships neither; [`carry_over`] and `bin/launch.sh` each copy them
/// into [`STAGING_DIR`].
pub const SHARED_FILES: [&str; 2] = ["config.txt", "installs.txt"];

/// The release this app installs. `key` names no `super::record` line:
/// [`current`] reports the version.
pub const APP: Source = Source {
    key: "app",
    name: "kfxdedrm-fe",
    repo: "huangziwei/kfxdedrm-fe",
    // The same release carries `kfxdedrm-koplugin-<tag>.zip` for the plugin.
    asset: |name| between(name, "kfxdedrm-fe-", "-kindle.zip").is_some(),
    version: |asset, tag| {
        between(asset, "kfxdedrm-fe-", "-kindle.zip")
            .unwrap_or(tag)
            .to_string()
    },
    releases: RELEASES_URL,
    asset_name: RELEASE_ASSET,
    // Names the archive's root: `extensions/kfxdedrm-fe/bin/launch.sh`. The
    // `documents/` tree and the LICENSE beside it fall outside and are skipped.
    marker: "bin/launch.sh",
    dest: EXTENSION_DIR,
    verify: |dir| runnable(&dir.join("bin")),
};

/// The version this build was compiled at, spelled `0.4.0` where a release tag
/// spells it `v0.4.0`. [`is_newer`] reads both.
pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// What [`update`] did, as the banner's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// This build is the newest release, or ahead of it.
    Current(String),
    /// Downloaded, proved and left in [`STAGING_DIR`].
    Staged(String),
    /// Nothing was installed.
    Failed(String),
}

impl Outcome {
    /// What the banner shows.
    pub fn message(&self) -> String {
        match self {
            Outcome::Current(version) => format!("kfxdedrm-fe {version}\nAlready the newest build"),
            Outcome::Staged(version) => {
                format!("kfxdedrm-fe {version} is ready\nClosing — open it again to finish")
            }
            Outcome::Failed(why) => format!("kfxdedrm-fe: {why}"),
        }
    }

    /// Whether `bin/launch.sh` has something to apply.
    pub fn is_staged(&self) -> bool {
        matches!(self, Outcome::Staged(_))
    }
}

//------------------------------------------------------------------------------
// Pure: which of two versions is later
//------------------------------------------------------------------------------

/// Whether `offered` names a later release than `installed`: dot-separated
/// numbers, an optional leading `v`, and a `-rc1` suffix sorting before the
/// same numbers without one. A string carrying no number is later than nothing.
pub fn is_newer(offered: &str, installed: &str) -> bool {
    ordering(offered, installed) == Some(Ordering::Greater)
}

fn ordering(offered: &str, installed: &str) -> Option<Ordering> {
    let (a, a_suffix) = parts(offered)?;
    let (b, b_suffix) = parts(installed)?;
    for i in 0..a.len().max(b.len()) {
        let at = |v: &[u32]| v.get(i).copied().unwrap_or(0);
        match at(&a).cmp(&at(&b)) {
            Ordering::Equal => {}
            other => return Some(other),
        }
    }
    // The release without a suffix sorts later.
    Some(a_suffix.is_empty().cmp(&b_suffix.is_empty()))
}

/// `version` as its numbers and whatever followed them: `v0.5.0-rc1` reads as
/// `([0, 5, 0], "-rc1")`. `None` when it opens with no number at all.
fn parts(version: &str) -> Option<(Vec<u32>, &str)> {
    let body = version.strip_prefix('v').unwrap_or(version);
    let (numbers, suffix) = match body.find(|c: char| c != '.' && !c.is_ascii_digit()) {
        Some(at) => body.split_at(at),
        None => (body, ""),
    };
    let numbers: Vec<u32> = numbers
        .split('.')
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().ok())
        .collect::<Option<_>>()?;
    (!numbers.is_empty()).then_some((numbers, suffix))
}

//------------------------------------------------------------------------------
// The whole flow
//------------------------------------------------------------------------------

/// Fetch the newest release and leave it staged for `bin/launch.sh`.
///
/// Blocking. `cancel` is read between chunks of the download.
pub fn update(say: &dyn Fn(String), cancel: &AtomicBool) -> Outcome {
    if crate::net::is_offline() {
        return Outcome::Failed("no route off this Kindle\nTurn Wi-Fi on".into());
    }

    let client = http::Client::new();
    let release = match super::available(&client, &APP) {
        Ok(release) => release,
        Err(why) => {
            log(format!(
                "update: by hand, unzip {RELEASE_ASSET} from {RELEASES_URL} over {EXTENSION_DIR}"
            ));
            return Outcome::Failed(format!("no release list ({why})"));
        }
    };

    if !is_newer(&release.version, current()) {
        return Outcome::Current(current().to_string());
    }

    match stage(&client, &release, say, cancel) {
        Ok(()) => {
            log(format!(
                "update: {} staged at {STAGING_DIR}",
                release.version
            ));
            Outcome::Staged(release.version)
        }
        Err(why) => {
            log(format!("update: {why}"));
            Outcome::Failed(why)
        }
    }
}

/// Download, prove and leave in [`STAGING_DIR`].
fn stage(
    client: &http::Client,
    release: &Release,
    say: &dyn Fn(String),
    cancel: &AtomicBool,
) -> Result<(), String> {
    let staging = PathBuf::from(STAGING_DIR);
    download_and_unpack(client, &APP, release, &staging, say, cancel)?;
    carry_over(Path::new(EXTENSION_DIR), &staging);

    say("Checking it runs here…".into());
    if !(APP.verify)(&staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err("that build does not run on this Kindle".into());
    }
    Ok(())
}

/// Every file directly under `from` that `to` does not hold, copied across.
/// The archive ships the app alone; [`SHARED_FILES`] and anything beside them
/// outlive the folder they sit in.
fn carry_over(from: &Path, to: &Path) {
    let Ok(entries) = fs::read_dir(from) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name() else {
            continue;
        };
        let kept = to.join(name);
        if !path.is_file() || kept.exists() {
            continue;
        }
        match fs::copy(&path, &kept) {
            Ok(_) => log(format!("update: kept {}", name.to_string_lossy())),
            Err(e) => log(format!(
                "update: {} did not carry over: {e}",
                path.display()
            )),
        }
    }
}

/// Whether a proven copy is waiting for `bin/launch.sh`.
pub fn staged() -> bool {
    staged_in(Path::new(STAGING_DIR))
}

fn staged_in(dir: &Path) -> bool {
    dir.join(APP.marker).is_file()
}

/// Whether any [`ABI_VARIANTS`] build under `dir` starts on this device.
pub fn runnable(dir: &Path) -> bool {
    ABI_VARIANTS.iter().any(|name| starts(&dir.join(name)))
}

/// One build, against [`VERSION_FLAG`]. A binary built for the other float ABI
/// fails to load and never reaches `main`.
fn starts(exe: &Path) -> bool {
    Command::new(exe)
        .arg(VERSION_FLAG)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::{ApiRelease, pick_release};
    use std::os::unix::fs::PermissionsExt;

    /// The release list, as the API served it.
    fn releases() -> Vec<ApiRelease> {
        serde_json::from_str(include_str!("../../tests/fixtures/releases-fe.json")).unwrap()
    }

    /// A scratch folder of this test's own.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kfxdedrm-fe-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A `bin/<name>` under `dir` exiting `code`.
    fn variant(dir: &Path, name: &str, code: i32) -> PathBuf {
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let exe = bin.join(name);
        fs::write(&exe, format!("#!/bin/sh\nexit {code}\n")).unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        exe
    }

    #[test]
    fn the_newest_release_is_taken_by_asset_name_not_by_tag() {
        let picked = pick_release(&releases(), &APP).unwrap();
        assert_eq!(picked.name, "kfxdedrm-fe-v0.4.0-kindle.zip");
        assert_eq!(picked.version, "v0.4.0");
        assert!(picked.sha.is_some_and(|s| s.ends_with(".sha256")));
    }

    #[test]
    fn the_plugin_asset_of_the_same_release_is_not_this_one() {
        assert!((APP.asset)("kfxdedrm-fe-v0.4.0-kindle.zip"));
        // One release carries both frontends; this asset is the app.
        assert!(!(APP.asset)("kfxdedrm-koplugin-v0.4.0.zip"));
        assert!(!(APP.asset)("kfxdedrm-fe-v0.4.0-kindle.zip.sha256"));
        assert!(!(APP.asset)("kfxdedrmmobi.zip"));
        assert!(!(APP.asset)("bokai-v0.1.3-kindle.zip"));
    }

    #[test]
    fn the_version_recorded_is_the_one_the_filename_carries() {
        assert_eq!(
            (APP.version)("kfxdedrm-fe-v0.4.0-kindle.zip", "v9.9.9"),
            "v0.4.0"
        );
        // An asset with nothing between the two halves falls back to the tag.
        assert_eq!((APP.version)("kfxdedrm-fe--kindle.zip", "v9.9.9"), "v9.9.9");
    }

    #[test]
    fn a_later_release_is_offered_and_this_build_is_not_offered_to_itself() {
        assert!(is_newer("v0.5.0", "0.4.0"));
        assert!(is_newer("v0.4.1", "0.4.0"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("v0.4.0", "0.4.0"));
        assert!(!is_newer("v0.3.0", "0.4.0"));
        // A downgrade is never offered, whichever side carries the `v`.
        assert!(!is_newer("0.3.0", "v0.4.0"));
        // The live list against the build in this tree.
        let newest = pick_release(&releases(), &APP).unwrap().version;
        assert!(!is_newer(&newest, current()), "{newest} over {}", current());
    }

    #[test]
    fn a_missing_component_reads_as_zero_and_a_suffix_loses_to_the_release() {
        assert!(!is_newer("v0.5", "0.5.0"));
        assert!(!is_newer("v0.5.0", "0.5"));
        assert!(is_newer("v0.5.1", "0.5"));
        assert!(is_newer("v0.5.0", "0.5.0-rc1"));
        assert!(!is_newer("v0.5.0-rc1", "0.5.0"));
        // Two suffixes are not ordered against each other.
        assert!(!is_newer("v0.5.0-rc2", "0.5.0-rc1"));
    }

    #[test]
    fn a_version_that_cannot_be_read_never_starts_an_update() {
        assert!(!is_newer("nightly", "0.4.0"));
        assert!(!is_newer("", "0.4.0"));
        assert!(!is_newer("v", "0.4.0"));
        assert!(!is_newer("v99.0.0", "nightly"));
        // Wider than a u32.
        assert!(!is_newer("v99999999999.0.0", "0.4.0"));
        assert_eq!(parts("v0.5.0-rc1"), Some((vec![0, 5, 0], "-rc1")));
        assert_eq!(parts("0.5"), Some((vec![0, 5], "")));
        assert_eq!(parts("rc1"), None);
    }

    #[test]
    fn this_build_names_a_version_that_reads_as_one() {
        assert!(parts(current()).is_some(), "{}", current());
        // Cargo carries no `v`; every tag in the list does.
        assert!(!current().starts_with('v'));
        assert!(releases().iter().all(|r| r.tag_name.starts_with('v')));
    }

    #[test]
    fn a_folder_resolves_as_staged_only_once_the_launch_script_is_in_it() {
        let dir = scratch("staged");
        assert!(!staged_in(&dir));
        fs::create_dir_all(dir.join("bin")).unwrap();
        assert!(!staged_in(&dir));
        fs::write(dir.join(APP.marker), "#!/bin/sh\n").unwrap();
        assert!(staged_in(&dir));
        // What `bin/launch.sh` tests for is what `stage` leaves behind.
        assert_eq!(APP.marker, "bin/launch.sh");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_download_is_proved_by_whichever_of_the_two_builds_starts() {
        let dir = scratch("runnable");
        assert!(!runnable(&dir.join("bin")));
        // Hard-float present but refused by the loader, soft-float running.
        variant(&dir, ABI_VARIANTS[0], 1);
        assert!(!runnable(&dir.join("bin")));
        variant(&dir, ABI_VARIANTS[1], 0);
        assert!(runnable(&dir.join("bin")));
        assert!((APP.verify)(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_settings_both_frontends_share_survive_the_folder_they_sit_in() {
        let installed = scratch("carry-from");
        let staging = scratch("carry-to");

        // What the running install holds, and what the archive ships.
        fs::write(
            installed.join("config.txt"),
            "scan_dir = /mnt/us/documents\n",
        )
        .unwrap();
        fs::write(installed.join("installs.txt"), "bokai = v0.1.3\n").unwrap();
        fs::write(installed.join("menu.json"), "old").unwrap();
        fs::create_dir_all(installed.join("bin")).unwrap();
        fs::write(staging.join("menu.json"), "new").unwrap();

        carry_over(&installed, &staging);

        assert_eq!(
            fs::read_to_string(staging.join("config.txt")).unwrap(),
            "scan_dir = /mnt/us/documents\n"
        );
        assert_eq!(
            fs::read_to_string(staging.join("installs.txt")).unwrap(),
            "bokai = v0.1.3\n"
        );
        // The release wins wherever it ships a file of the same name.
        assert_eq!(
            fs::read_to_string(staging.join("menu.json")).unwrap(),
            "new"
        );
        // `bin/` is the archive's alone.
        assert!(!staging.join("bin").exists());

        let _ = fs::remove_dir_all(&installed);
        let _ = fs::remove_dir_all(&staging);
    }

    #[test]
    fn a_first_install_with_nothing_to_carry_over_is_not_an_error() {
        let staging = scratch("carry-none");
        carry_over(Path::new("/nonexistent/kfxdedrm-fe"), &staging);
        assert_eq!(fs::read_dir(&staging).unwrap().count(), 0);
        let _ = fs::remove_dir_all(&staging);
    }

    #[test]
    fn every_outcome_says_which_build_it_is_about() {
        assert!(
            Outcome::Staged("v0.5.0".into())
                .message()
                .contains("v0.5.0")
        );
        assert!(Outcome::Staged("v0.5.0".into()).is_staged());
        assert!(Outcome::Current("0.4.0".into()).message().contains("0.4.0"));
        assert!(!Outcome::Current("0.4.0".into()).is_staged());
        let failed = Outcome::Failed("no release list (offline)".into());
        assert!(failed.message().contains("no release list"));
        assert!(!failed.is_staged());
    }

    /// The v0.4.0 assets, as `unzip -Z1` lists them.
    ///
    /// Both frontends ship in one release, and the app's archive opens two
    /// trees deep with two more entries beside them.
    #[test]
    fn the_published_archive_unpacks_out_of_the_folder_it_replaces() {
        let published = [
            "extensions/",
            "extensions/kfxdedrm-fe/",
            "extensions/kfxdedrm-fe/menu.json",
            "extensions/kfxdedrm-fe/bin/",
            "extensions/kfxdedrm-fe/bin/launch.sh",
            "extensions/kfxdedrm-fe/bin/kfxdedrm-fe",
            "extensions/kfxdedrm-fe/bin/kfxdedrm-fe-armsf",
            "extensions/kfxdedrm-fe/config.xml",
            "documents/",
            "documents/KFXDeDRM.sh",
            "LICENSE",
        ];
        assert_eq!(
            crate::install::archive::prefix_for(published, APP.marker),
            Some("extensions/kfxdedrm-fe/".to_string())
        );
        // The home-screen tile and the LICENSE fall outside that prefix.
        // `archive::unpack` writes neither.
        let prefix = "extensions/kfxdedrm-fe/";
        let inside: Vec<&str> = published
            .iter()
            .filter(|p| p.starts_with(prefix) && !p.ends_with('/'))
            .copied()
            .collect();
        assert_eq!(
            inside,
            [
                "extensions/kfxdedrm-fe/menu.json",
                "extensions/kfxdedrm-fe/bin/launch.sh",
                "extensions/kfxdedrm-fe/bin/kfxdedrm-fe",
                "extensions/kfxdedrm-fe/bin/kfxdedrm-fe-armsf",
                "extensions/kfxdedrm-fe/config.xml",
            ]
        );
        // Neither is `config.txt` nor `installs.txt`; `carry_over` supplies those.
        assert!(!published.iter().any(|p| p.ends_with(".txt")));
    }

    /// `bin/launch.sh` probes the same builds with the same flag, and applies
    /// what [`stage`] leaves in [`STAGING_DIR`]. Nothing else keeps the two
    /// files in step.
    #[test]
    fn the_launch_script_names_the_same_builds_and_the_same_folders() {
        const LAUNCH: &str = include_str!("../../../device/extensions/kfxdedrm-fe/bin/launch.sh");
        for line in [
            format!("BIN_DIR={EXTENSION_DIR}/bin"),
            format!("VARIANTS=\"{}\"", ABI_VARIANTS.join(" ")),
        ] {
            assert!(LAUNCH.contains(&line), "launch.sh carries no `{line}`");
        }
        assert!(LAUNCH.contains(VERSION_FLAG));
        assert!(LAUNCH.contains(STAGING_DIR));
        // The old tree it clears is beside the two, not one of them.
        assert!(LAUNCH.contains(&format!("{EXTENSION_DIR}.old")));
        // And it copies the shared files again, over what `stage` carried.
        for name in SHARED_FILES {
            assert!(LAUNCH.contains(name), "launch.sh does not carry {name}");
        }
    }

    /// [`SHARED_FILES`] is the two paths the rest of the crate names.
    #[test]
    fn the_shared_files_are_the_ones_the_two_frontends_write() {
        let named = |path: &str| {
            path.rsplit('/')
                .next()
                .map(str::to_string)
                .unwrap_or_default()
        };
        assert_eq!(named(crate::install::record::PATH), SHARED_FILES[1]);
        // Both sit directly under the folder an update replaces.
        assert!(crate::install::record::PATH.starts_with(EXTENSION_DIR));
        assert_eq!(
            crate::install::record::PATH,
            format!("{EXTENSION_DIR}/{}", SHARED_FILES[1])
        );
    }
}
