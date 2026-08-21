//! `install::archive` over the two archives GitHub actually serves.
//!
//! They are several megabytes and are not in the repository.
//! `koplugin/spec/run.sh` fetches them into `koplugin/spec/cache/` for the
//! plugin's own suite, and this reads the same two files. Without them these
//! tests report what they skipped and pass — the reader's own unit tests cover
//! it against an archive they build themselves.

use std::path::{Path, PathBuf};

use kfxdedrm_fe_native::engine;
use kfxdedrm_fe_native::install::{self, archive};

/// Where `koplugin/spec/run.sh` leaves them.
fn cached(name: &str) -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("koplugin/spec/cache")
        .join(name);
    path.is_file().then_some(path)
}

fn tmpdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kfxdedrm-fe-real-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn size_of(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or_default()
}

#[test]
fn the_engine_archive_unpacks_into_the_folder_the_probe_walks() {
    let Some(zip) = cached("kfxdedrmmobi.zip") else {
        eprintln!("skipped: koplugin/spec/cache/kfxdedrmmobi.zip is not there");
        return;
    };
    let source = install::source("engine").unwrap();
    let dest = tmpdir("engine");

    // Nine entries in the archive, two of which are the folders themselves.
    let written = archive::unpack(&zip, source.marker, &dest).unwrap();
    assert_eq!(written, 7);

    // Every ABI variant `engine::locate_in` would try is where it looks.
    for path in engine::variant_paths(&dest.join("bin")) {
        assert!(path.is_file(), "{}", path.display());
    }
    assert!(dest.join("bin/run_cmd.sh").is_file());
    assert!(dest.join("config.xml").is_file());
    // The archive's own folder name is not repeated inside the target.
    assert!(!dest.join("kfxdedrm").exists());
    // A build comes out whole, which is the CRC check having held.
    assert_eq!(size_of(&dest.join("bin/kfxdedrmhf_c11")), 795072);

    let _ = std::fs::remove_dir_all(&dest);
}

#[test]
fn the_bokai_archive_unpacks_out_of_a_deeper_root() {
    let Some(zip) = cached("bokai.zip") else {
        eprintln!("skipped: koplugin/spec/cache/bokai.zip is not there");
        return;
    };
    let source = install::source("bokai").unwrap();
    let dest = tmpdir("bokai");

    assert_eq!(archive::unpack(&zip, source.marker, &dest).unwrap(), 2);
    assert_eq!(size_of(&dest.join("bin/bokai")), 5274348);
    assert!(dest.join("config.xml").is_file());
    // The two folders above it are gone.
    assert!(!dest.join("extensions").exists());

    let _ = std::fs::remove_dir_all(&dest);
}

#[test]
fn an_archive_that_is_not_the_one_asked_for_is_refused() {
    let Some(zip) = cached("bokai.zip") else {
        eprintln!("skipped: koplugin/spec/cache/bokai.zip is not there");
        return;
    };
    let engine = install::source("engine").unwrap();
    let dest = tmpdir("wrong");

    let err = archive::unpack(&zip, engine.marker, &dest).unwrap_err();
    assert!(err.to_string().contains(engine.marker), "{err}");
    assert!(
        !dest.exists(),
        "nothing may land before the marker is found"
    );
}
