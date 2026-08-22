//! Fetching the kfxdedrm engine and the bokai add-on from their own GitHub
//! releases.
//!
//! Neither ships with this app — one is someone else's project and both are
//! several megabytes of ARM binary — so what is offered here is the download
//! the first screen otherwise asks for by hand.
//!
//! Two things about those repositories decide the shape of this:
//!
//! - **`/releases/latest` is the wrong endpoint.** Every DeDRM_tools release
//!   that carries `kfxdedrmmobi.zip` is marked a prerelease, which that
//!   endpoint skips, and a sidle tag is published before its assets finish
//!   uploading. [`pick_release`] walks the release list instead and takes the
//!   newest one that actually holds a matching asset.
//! - **The two zips have different roots** — `kfxdedrm/…` against
//!   `extensions/bokai/…` — and neither matches the folder it installs into.
//!   So no path inside an archive is hardcoded: `archive::prefix_for` finds
//!   the entry ending in the source's [`Source::marker`] and everything under
//!   that entry's prefix is what gets extracted.
//!
//! An install is staged beside its destination and has to prove itself — the
//! engine by `engine::locate_in`, bokai by `convert::locate_in` — before it
//! replaces what is already there. A download that arrives corrupt, or built
//! for another ABI, therefore costs nothing.
//!
//! This is a port of the KOReader plugin's `lib/install.lua`. The two drive
//! the same repositories with the same asset patterns and the same markers,
//! and [`record`] is one file they share, so neither fetches what the other
//! already has; behaviour that differs between them is a bug in one of the two.

pub mod archive;
pub mod http;
pub mod record;

use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::log;
use crate::{convert, engine};

/// Releases to look through, newest first. Well past the depth at which the
/// engine last shipped its asset.
const RELEASES_PER_PAGE: u32 = 30;

/// What can be installed. [`Source::verify`] is what a staged copy has to pass.
pub struct Source {
    /// The key its release is recorded under in [`record`].
    pub key: &'static str,
    /// What the banner calls it.
    pub name: &'static str,
    pub repo: &'static str,
    /// Which asset of a release this installs, by filename.
    pub asset: fn(&str) -> bool,
    /// The releases page, and the asset named the way someone would look for
    /// it there — `*` standing for a version the filename carries.
    ///
    /// Neither is used to fetch anything: they are what [`by_hand`] logs when
    /// fetching fails, and the only thing anywhere that says how to install
    /// this without a network.
    pub releases: &'static str,
    pub asset_name: &'static str,
    /// An entry that names the archive's own root — see `archive::prefix_for`.
    pub marker: &'static str,
    /// Where the unpacked copy lands.
    pub dest: &'static str,
    /// Does the copy staged at this folder actually run on this device?
    pub verify: fn(&Path) -> bool,
}

pub const SOURCES: [Source; 2] = [
    Source {
        key: "engine",
        name: "kfxdedrm",
        repo: "Satsuoni/DeDRM_tools",
        // `kfxdedrm.zip` and `kfxdedrm_kual.zip` are older, KFX-only assets.
        asset: |name| name == engine::RELEASE_ASSET,
        releases: engine::RELEASES_URL,
        asset_name: engine::RELEASE_ASSET,
        // Names the archive's root: `kfxdedrm/bin/kfxdedrmhf_c11`.
        marker: "bin/kfxdedrmhf_c11",
        dest: engine::EXTENSION_DIR,
        verify: |dir| engine::locate_in(&dir.join("bin")).is_ok(),
    },
    Source {
        key: "bokai",
        name: "bokai",
        repo: "huangziwei/sidle",
        // The version rides the filename, so no one name belongs here.
        asset: |name| name.starts_with("bokai-") && name.ends_with("-kindle.zip"),
        releases: convert::RELEASES_URL,
        asset_name: convert::RELEASE_ASSET,
        // Names the archive's root: `extensions/bokai/bin/bokai`.
        marker: "bin/bokai",
        dest: convert::EXTENSION_DIR,
        verify: |dir| convert::locate_in(&dir.join("bin")).is_some(),
    },
];

pub fn source(key: &str) -> Option<&'static Source> {
    SOURCES.iter().find(|s| s.key == key)
}

/// The release a [`Source`] would install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    /// The asset to download.
    pub url: String,
    pub name: String,
    /// A `.sha256` sidecar, when the release publishes one. bokai does;
    /// DeDRM_tools does not.
    pub sha: Option<String>,
}

/// One release as the GitHub API serves it, cut to what [`pick_release`] reads.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ApiRelease {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    /// Not read by [`pick_release`]: it is why `/releases/latest` is useless
    /// here, which the tests assert and rely on.
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<ApiAsset>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ApiAsset {
    pub name: String,
    pub browser_download_url: String,
}

/// One line for the banner while an install runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Which add-on this is, 1-based, and how many there are in all.
    pub nth: usize,
    pub of: usize,
    pub name: &'static str,
    /// What is happening now.
    pub detail: String,
}

impl Step {
    /// The banner's title line.
    pub fn title(&self) -> String {
        format!("{}  ({}/{})", self.name, self.nth, self.of)
    }
}

//------------------------------------------------------------------------------
// Pure: what to fetch, and what to check it against
//------------------------------------------------------------------------------

/// The newest release in `releases` carrying an asset `source` names.
pub fn pick_release(releases: &[ApiRelease], source: &Source) -> Option<Release> {
    for release in releases {
        if release.draft {
            continue;
        }
        let Some(found) = release.assets.iter().rfind(|a| (source.asset)(&a.name)) else {
            continue;
        };
        let sidecar = format!("{}.sha256", found.name);
        return Some(Release {
            tag: release.tag_name.clone(),
            url: found.browser_download_url.clone(),
            name: found.name.clone(),
            sha: release
                .assets
                .iter()
                .find(|a| a.name == sidecar)
                .map(|a| a.browser_download_url.clone()),
        });
    }
    None
}

/// The `sha256sum`-style line for `name`, or the whole file when it carries a
/// bare digest.
pub fn digest_from(text: &str, name: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        let mut parts = line.splitn(2, char::is_whitespace);
        let digest = parts.next().unwrap_or_default();
        if !is_sha256(digest) {
            continue;
        }
        match parts.next() {
            // A file holding nothing but the digest.
            None => return Some(digest.to_ascii_lowercase()),
            Some(rest) => {
                // `sha256sum -b` marks a binary with a `*`, and a digest taken
                // from a build directory names the file by its whole path.
                let named = rest.trim_start().trim_start_matches('*');
                if named == name || named.rsplit('/').next() == Some(name) {
                    return Some(digest.to_ascii_lowercase());
                }
            }
        }
    }
    None
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `got` against `total`, for the banner.
fn transferred(got: u64, total: Option<u64>) -> String {
    match total {
        Some(total) if total > 0 => format!("{}%", got * 100 / total),
        // No `Content-Length`: the count is all there is to say.
        _ => format!("{:.1} MB", got as f64 / (1024.0 * 1024.0)),
    }
}

//------------------------------------------------------------------------------
// The whole flow
//------------------------------------------------------------------------------

/// Fetch or update every [`SOURCES`] entry, one summary line each.
///
/// Blocking, and meant for a worker thread: `cancel` is what the panel's
/// Cancel button sets, and it is read between chunks of a download and between
/// one add-on and the next.
pub fn install_all(record_path: &Path, say: &dyn Fn(Step), cancel: &AtomicBool) -> Vec<String> {
    if crate::net::is_offline() {
        return vec!["No route off this Kindle.".into(), "Turn Wi-Fi on.".into()];
    }

    let client = http::Client::new();
    let mut record = record::Record::load(record_path);
    let mut lines = Vec::new();

    for (i, source) in SOURCES.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            lines.push(format!("{}: cancelled", source.name));
            continue;
        }
        let step = |detail: String| {
            say(Step {
                nth: i + 1,
                of: SOURCES.len(),
                name: source.name,
                detail,
            })
        };
        step("Asking GitHub…".into());
        lines.push(install_one(&client, source, &mut record, &step, cancel));
    }

    if let Err(e) = record.store(record_path) {
        log(format!("installs.txt: {e}"));
    }
    lines
}

/// One add-on: what it is now, or why it is not. The line is the banner's.
fn install_one(
    client: &http::Client,
    source: &Source,
    record: &mut record::Record,
    step: &dyn Fn(String),
    cancel: &AtomicBool,
) -> String {
    let release = match available(client, source) {
        Ok(release) => release,
        Err(why) => {
            by_hand(source);
            return format!("{}: no release list ({why})", source.name);
        }
    };

    // The record alone is not enough: it names a release that was installed,
    // not one that still runs here.
    if record.get(source.key) == Some(release.tag.as_str())
        && (source.verify)(Path::new(source.dest))
    {
        return format!("{}: already at {}", source.name, release.tag);
    }

    match fetch(client, source, &release, step, cancel) {
        Ok(()) => {
            record.set(source.key, &release.tag);
            log(format!(
                "installed {} {} into {}",
                source.name, release.tag, source.dest
            ));
            format!("{}: installed {}", source.name, release.tag)
        }
        Err(why) => {
            by_hand(source);
            format!("{}: {why}", source.name)
        }
    }
}

/// Where to get `source` when this could not. The banner has no room for a URL
/// and no browser to open one, so the log is where it goes.
fn by_hand(source: &Source) {
    log(format!(
        "{}: by hand, unzip {} from {} into {}",
        source.name, source.asset_name, source.releases, source.dest
    ));
}

/// The release `source` would install.
pub fn available(client: &http::Client, source: &Source) -> Result<Release, &'static str> {
    let url = format!(
        "https://api.github.com/repos/{}/releases?per_page={RELEASES_PER_PAGE}",
        source.repo
    );
    let body = client
        .text(&url, "application/vnd.github+json")
        .map_err(|e| {
            log(format!("{}: {e}", source.name));
            e.hint()
        })?;

    let releases: Vec<ApiRelease> = serde_json::from_str(&body).map_err(|e| {
        log(format!("{}: unreadable release list: {e}", source.name));
        "bad reply"
    })?;

    pick_release(&releases, source).ok_or("no release has it")
}

/// Download, unpack, prove and swap in.
///
/// Nothing already in place is touched until a staged copy has run on this
/// device, so a failure anywhere here leaves the previous install working.
fn fetch(
    client: &http::Client,
    source: &Source,
    release: &Release,
    step: &dyn Fn(String),
    cancel: &AtomicBool,
) -> Result<(), String> {
    let dest = PathBuf::from(source.dest);
    let staging = PathBuf::from(format!("{}.new", source.dest));
    let zip = PathBuf::from(format!("{}.new.zip", source.dest));
    let _ = fs::remove_dir_all(&staging);
    let _ = fs::remove_file(&zip);

    step("Downloading…".into());
    // The panel repaints on every step it is handed and each one is a full
    // banner; a percentage that moves by 2 is often enough to look live.
    let last = std::cell::Cell::new(u64::MAX);
    let progress = |got: u64, total: Option<u64>| {
        let mark = match total {
            Some(total) if total > 0 => got * 50 / total,
            _ => got / (512 * 1024),
        };
        if mark != last.get() {
            last.set(mark);
            step(format!("Downloading  {}", transferred(got, total)));
        }
    };
    if let Err(e) = client.download(&release.url, &zip, cancel, &progress) {
        log(format!("{}: {e}", source.name));
        return Err(match e {
            http::Error::Cancelled => "cancelled".into(),
            other => format!("download failed ({})", other.hint()),
        });
    }

    if let Some(sidecar) = &release.sha {
        check_digest(client, sidecar, &release.name, &zip).inspect_err(|_| {
            let _ = fs::remove_file(&zip);
        })?;
    }

    step("Unpacking…".into());
    let unpacked = archive::unpack(&zip, source.marker, &staging);
    let _ = fs::remove_file(&zip);
    match unpacked {
        Ok(written) => log(format!(
            "{}: {written} files into {:?}",
            source.name, staging
        )),
        Err(e) => {
            log(format!("{}: {e}", source.name));
            let _ = fs::remove_dir_all(&staging);
            return Err("the download is not a usable archive".into());
        }
    }

    // The archive may or may not carry Unix modes depending on what wrote it,
    // so nothing under bin/ is assumed to have come out executable.
    mark_executable(&staging.join("bin"));

    step("Checking it runs here…".into());
    if !(source.verify)(&staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err("that build does not run on this Kindle".into());
    }

    swap_in(&staging, &dest)
}

/// The staged copy into place, keeping the old one until the move lands.
fn swap_in(staging: &Path, dest: &Path) -> Result<(), String> {
    let previous = PathBuf::from(format!("{}.old", dest.display()));
    let _ = fs::remove_dir_all(&previous);

    let had_one = dest.is_dir();
    if had_one && !move_dir(dest, &previous) {
        let _ = fs::remove_dir_all(staging);
        return Err("cannot move the old copy aside".into());
    }
    if !move_dir(staging, dest) {
        // Put back whatever was working before failing. Failing that too
        // leaves the old copy at `.old`, which the log has to say.
        if had_one && !move_dir(&previous, dest) {
            log(format!("the previous copy is at {}", previous.display()));
        }
        let _ = fs::remove_dir_all(staging);
        return Err("cannot move the new copy into place".into());
    }
    let _ = fs::remove_dir_all(&previous);
    Ok(())
}

/// `from` to `to`, through `mv` when `rename` will not have it.
///
/// The two paths share a parent, which is the case `rename` handles on the
/// FAT partition these live on. `mv` is the fallback for the case it does not.
fn move_dir(from: &Path, to: &Path) -> bool {
    if fs::rename(from, to).is_ok() {
        return true;
    }
    Command::new("mv")
        .arg(from)
        .arg(to)
        .status()
        .is_ok_and(|s| s.success())
}

/// The downloaded file against the digest the release publishes for it.
///
/// A sidecar that cannot be read is not a mismatch: DeDRM_tools publishes none
/// at all, and the gate that actually holds either way is that the staged copy
/// has to run.
fn check_digest(
    client: &http::Client,
    sidecar: &str,
    name: &str,
    zip: &Path,
) -> Result<(), String> {
    let Ok(text) = client.text(sidecar, "text/plain") else {
        log("checksum: the sidecar could not be read");
        return Ok(());
    };
    let Some(want) = digest_from(&text, name) else {
        log("checksum: the sidecar names no digest for this file");
        return Ok(());
    };
    let Some(got) = digest_of(zip) else {
        log("checksum: the download could not be read back");
        return Ok(());
    };
    if want != got {
        log(format!("checksum: wanted {want}, got {got}"));
        return Err("the download does not match its checksum".into());
    }
    log("checksum: matched");
    Ok(())
}

/// SHA-256 of `path`, hex.
fn digest_of(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};

    let mut file = fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
}

/// Every file directly under `dir`, executable. Best effort: the partition is
/// FAT and its modes come from the mount as often as from the file.
fn mark_executable(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        if meta.is_file() {
            let mut perms = meta.permissions();
            perms.set_mode(perms.mode() | 0o755);
            let _ = fs::set_permissions(&path, perms);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> &'static Source {
        source("engine").unwrap()
    }

    fn bokai() -> &'static Source {
        source("bokai").unwrap()
    }

    /// The two release lists, as the API served them.
    fn dedrm() -> Vec<ApiRelease> {
        serde_json::from_str(include_str!("../../tests/fixtures/releases-dedrm.json")).unwrap()
    }

    fn sidle() -> Vec<ApiRelease> {
        serde_json::from_str(include_str!("../../tests/fixtures/releases-sidle.json")).unwrap()
    }

    #[test]
    fn both_sources_are_named_and_nothing_else_is() {
        assert_eq!(SOURCES.len(), 2);
        assert!(source("nope").is_none());
        assert_eq!(engine().dest, engine::EXTENSION_DIR);
        assert_eq!(bokai().dest, convert::EXTENSION_DIR);
    }

    #[test]
    fn the_engine_comes_off_the_newest_release_carrying_its_asset() {
        let picked = pick_release(&dedrm(), engine()).unwrap();
        assert_eq!(picked.tag, "v10.0.30");
        assert_eq!(picked.name, engine::RELEASE_ASSET);
        assert!(picked.url.contains(engine::RELEASE_ASSET));
        // DeDRM_tools publishes no checksum beside it.
        assert_eq!(picked.sha, None);
    }

    /// Why `/releases/latest` is the wrong endpoint: it returns the newest
    /// release that is *not* a prerelease, and no such release has ever
    /// carried the engine.
    #[test]
    fn no_release_that_latest_would_return_carries_the_engine() {
        let list = dedrm();
        let carrying: Vec<&ApiRelease> = list
            .iter()
            .filter(|r| r.assets.iter().any(|a| (engine().asset)(&a.name)))
            .collect();
        assert!(carrying.len() > 1, "{}", carrying.len());
        for release in &carrying {
            assert!(release.prerelease, "{}", release.tag_name);
        }
    }

    #[test]
    fn bokai_is_taken_by_asset_pattern_rather_than_by_version() {
        let list = sidle();
        let picked = pick_release(&list, bokai()).unwrap();
        // Whichever release is newest at the time, by the asset it holds.
        let (tag, name) = list
            .iter()
            .flat_map(|r| r.assets.iter().map(move |a| (&r.tag_name, &a.name)))
            .find(|(_, name)| (bokai().asset)(name))
            .unwrap();
        assert_eq!(&picked.tag, tag);
        assert_eq!(&picked.name, name);
        // And that one does publish a checksum.
        assert!(picked.sha.is_some_and(|s| s.ends_with(".sha256")));
    }

    #[test]
    fn a_tag_published_before_its_assets_is_passed_over() {
        let mut list = vec![ApiRelease {
            tag_name: "bokai-v9.9.9".into(),
            draft: false,
            prerelease: false,
            assets: Vec::new(),
        }];
        let whole = pick_release(&sidle(), bokai()).unwrap();
        list.extend(sidle());
        assert_eq!(pick_release(&list, bokai()), Some(whole));
    }

    #[test]
    fn a_draft_and_a_list_with_nothing_in_it_pick_nothing() {
        let asset = ApiAsset {
            name: engine::RELEASE_ASSET.into(),
            browser_download_url: "u".into(),
        };
        let draft = ApiRelease {
            tag_name: "v2".into(),
            draft: true,
            prerelease: true,
            assets: vec![asset.clone()],
        };
        assert_eq!(pick_release(&[draft], engine()), None);
        assert_eq!(pick_release(&[], engine()), None);

        let unrelated = ApiRelease {
            tag_name: "v1".into(),
            draft: false,
            prerelease: false,
            assets: vec![ApiAsset {
                name: "source.zip".into(),
                browser_download_url: "u".into(),
            }],
        };
        assert_eq!(pick_release(&[unrelated], engine()), None);
    }

    #[test]
    fn each_source_names_the_page_its_release_comes_from() {
        // One spelling of each: the two modules own these strings and `ui`
        // draws the engine's, so a second copy here could drift from both.
        assert_eq!(engine().releases, engine::RELEASES_URL);
        assert_eq!(engine().asset_name, engine::RELEASE_ASSET);
        assert_eq!(bokai().releases, convert::RELEASES_URL);
        assert_eq!(bokai().asset_name, convert::RELEASE_ASSET);
        // The name a person would look for is the one the matcher accepts,
        // give or take the version bokai's carries.
        assert!((engine().asset)(engine().asset_name));
        assert!(bokai().asset_name.contains('*'));
    }

    #[test]
    fn each_source_names_only_its_own_asset() {
        assert!((engine().asset)("kfxdedrmmobi.zip"));
        // The KFX-only assets of the same release.
        assert!(!(engine().asset)("kfxdedrm.zip"));
        assert!(!(engine().asset)("kfxdedrm_kual.zip"));
        assert!(!(engine().asset)("kfxdedrmmobi.zip.sha256"));

        assert!((bokai().asset)("bokai-v0.1.3-kindle.zip"));
        assert!((bokai().asset)("bokai-v0.1.10-kindle.zip"));
        assert!(!(bokai().asset)("bokai-v0.1.3-kindle.zip.sha256"));
        assert!(!(bokai().asset)("sidle-v0.1.3-kindle.zip"));
    }

    #[test]
    fn a_digest_is_read_off_whichever_shape_the_sidecar_takes() {
        let d = "a".repeat(64);
        assert_eq!(
            digest_from(
                &format!("{d}  bokai-v0.1.2-kindle.zip"),
                "bokai-v0.1.2-kindle.zip"
            ),
            Some(d.clone())
        );
        // `sha256sum -b` marks a binary file.
        assert_eq!(
            digest_from(&format!("{d} *file.zip"), "file.zip"),
            Some(d.clone())
        );
        // A digest taken in a build directory names the whole path.
        assert_eq!(
            digest_from(&format!("{d}  /tmp/build/file.zip"), "file.zip"),
            Some(d.clone())
        );
        // A file holding nothing but the digest is for whatever it came with.
        assert_eq!(
            digest_from(&format!("{d}\n"), "anything.zip"),
            Some(d.clone())
        );
        assert_eq!(
            digest_from(&format!("{}  f.zip", "A".repeat(64)), "f.zip"),
            Some("a".repeat(64))
        );
        assert_eq!(digest_from("abc  f.zip", "f.zip"), None);
        assert_eq!(digest_from("", "f.zip"), None);
        // A sidecar for a different file is not this file's digest.
        assert_eq!(digest_from(&format!("{d}  other.zip"), "f.zip"), None);
    }

    #[test]
    fn the_transferred_line_prefers_a_percentage_and_falls_back_to_a_count() {
        assert_eq!(transferred(0, Some(200)), "0%");
        assert_eq!(transferred(100, Some(200)), "50%");
        assert_eq!(transferred(200, Some(200)), "100%");
        // No `Content-Length`, or one that says nothing.
        assert_eq!(transferred(1024 * 1024, None), "1.0 MB");
        assert_eq!(transferred(1024 * 1024, Some(0)), "1.0 MB");
    }

    #[test]
    fn a_file_hashes_to_what_sha256sum_would_say() {
        let path = std::env::temp_dir().join("kfxdedrm-fe-digest.txt");
        fs::write(&path, b"abc").unwrap();
        assert_eq!(
            digest_of(&path).as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        let _ = fs::remove_file(&path);
        assert_eq!(digest_of(Path::new("/nonexistent/kfxdedrm-fe")), None);
    }

    #[test]
    fn a_step_names_which_add_on_it_is_of_how_many() {
        let step = Step {
            nth: 2,
            of: 2,
            name: "bokai",
            detail: "Unpacking…".into(),
        };
        assert_eq!(step.title(), "bokai  (2/2)");
    }
}
