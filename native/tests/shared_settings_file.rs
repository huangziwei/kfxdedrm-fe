//! The two files both frontends write, against the fixtures the KOReader
//! plugin's own suite reads.
//!
//! `koplugin/` writes the settings file and the install record too, at the
//! same two paths, so that one device carries one set of folders and switches
//! and one record of what is installed whichever frontend is running. The
//! renderers have to agree byte for byte: a difference means each frontend
//! rewrites the other's file every time it saves.
//!
//! These fixtures are the contract. Changing a format means changing them,
//! which fails `koplugin/spec` until its renderer is changed to match.

use std::path::{Path, PathBuf};

use kfxdedrm_fe_native::config::Config;
use kfxdedrm_fe_native::install::record::Record;

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("koplugin/spec/fixtures")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn the_default_settings_file_is_what_the_plugin_expects() {
    assert_eq!(Config::default().render(), fixture("config-default.txt"));
}

#[test]
fn a_file_with_no_folder_selected_is_what_the_plugin_expects() {
    let cfg = Config {
        scan_dirs: Vec::new(),
        types_kfx: false,
        types_mobi: true,
        pack_kfx: true,
        convert_epub: true,
        show_done: false,
    };
    assert_eq!(cfg.render(), fixture("config-no-folder.txt"));
}

#[test]
fn the_fixtures_read_back_as_the_settings_that_wrote_them() {
    assert_eq!(
        Config::parse(&fixture("config-default.txt")),
        Config::default()
    );
    assert_eq!(
        Config::parse(&fixture("config-no-folder.txt")).scan_dirs,
        Vec::<PathBuf>::new()
    );
}

#[test]
fn the_install_record_is_what_the_plugin_expects() {
    let mut record = Record::default();
    record.set("bokai", "bokai-v0.1.3");
    record.set("engine", "v10.0.30");
    assert_eq!(record.render(), fixture("installs.txt"));
}

#[test]
fn the_record_fixture_reads_back_as_the_releases_that_wrote_it() {
    let record = Record::parse(&fixture("installs.txt"));
    assert_eq!(record.get("engine"), Some("v10.0.30"));
    assert_eq!(record.get("bokai"), Some("bokai-v0.1.3"));
}
