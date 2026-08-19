//! [`scan::scan_in`] and [`engine::locate_in`] over real directory trees.
//!
//! The rules cover which files sit beside which other files. [`Tree`] roots
//! under `CARGO_TARGET_TMPDIR`, which cargo hands to an integration test and
//! cleans with the build tree.
//!
//! [`palmdb`] is written from the PalmDB format. A builder shared with
//! `mobi.rs` lets a wrong offset in one cancel a wrong offset in the other.

use std::path::{Path, PathBuf};

use kfxdedrm_fe_native::config::Config;
use kfxdedrm_fe_native::engine::{self, Format, Missing};
use kfxdedrm_fe_native::scan::{self, Book};

/// A PalmDB declaring `enc` at record 0: a 78-byte header, one 8-byte
/// record-list entry, then record 0.
fn palmdb(type_creator: &[u8; 8], enc: u16) -> Vec<u8> {
    let mut v = vec![0u8; 86];
    v[60..68].copy_from_slice(type_creator);
    v[76..78].copy_from_slice(&1u16.to_be_bytes()); // record count
    v[78..82].copy_from_slice(&86u32.to_be_bytes()); // record 0 begins at 86
    v.extend_from_slice(&[0u8; 12]); // compression .. record size
    v.extend_from_slice(&enc.to_be_bytes()); // encryption_type
    v
}

/// A scratch tree under `CARGO_TARGET_TMPDIR`, removed on drop.
struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join(tag);
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        Tree(base)
    }

    fn dir(&self, rel: &str) -> PathBuf {
        let p = self.0.join(rel);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// A `.kfx`, with the `.sdr` voucher sidecar when `voucher`.
    fn kfx(&self, dir: &Path, stem: &str, voucher: bool) {
        std::fs::write(dir.join(format!("{stem}.kfx")), b"kfx").unwrap();
        if voucher {
            let assets = dir.join(format!("{stem}.sdr")).join("assets");
            std::fs::create_dir_all(&assets).unwrap();
            std::fs::write(assets.join("voucher"), b"v").unwrap();
        }
    }

    /// A [`palmdb`] at `dir/name`.
    fn mobi(&self, dir: &Path, name: &str, enc: u16) {
        std::fs::write(dir.join(name), palmdb(b"BOOKMOBI", enc)).unwrap();
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A default `Config` writing into `out`. Each test passes its own roots to
/// [`scan::scan_in`], leaving the on-device paths out.
fn cfg_out(out: &Path) -> Config {
    Config {
        out_dir: out.to_path_buf(),
        ..Config::default()
    }
}

/// [`scan::scan_in`] over one root, no thumbnail cache.
fn scan_one(root: &Path, cfg: &Config) -> Vec<Book> {
    scan::scan_in(&[root.to_path_buf()], cfg, Path::new("/nonexistent"))
}

#[test]
fn a_kfx_needs_its_voucher_to_be_listed() {
    let t = Tree::new("kfx");
    let items = t.dir("Items01");
    let out = t.dir("out");
    t.kfx(&items, "Good Book_ Subtitle_B000O76ON6", true);
    t.kfx(&items, "Half Book_B000FC1BQK", false); // still downloading
    // An AppleDouble shadow sits beside the same voucher; its name rules it out.
    std::fs::write(items.join("._Good Book_ Subtitle_B000O76ON6.kfx"), b"x").unwrap();

    let found = scan_one(&items, &cfg_out(&out));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "Good Book: Subtitle");
    assert_eq!(found[0].asin.as_deref(), Some("B000O76ON6"));
    assert_eq!(found[0].format, Format::Kfx);
    assert!(!found[0].done);
}

#[test]
fn a_drm_free_sideload_is_never_listed() {
    let t = Tree::new("mobi");
    let docs = t.dir("documents");
    let out = t.dir("out");
    t.mobi(&docs, "Purchased.azw3", 2); // Mobipocket DRM
    t.mobi(&docs, "Old DRM.mobi", 1); // legacy DRM
    t.mobi(&docs, "My Own Book.azw3", 0); // nothing to strip
    std::fs::write(docs.join("Notes.epub"), b"not a mobi").unwrap();
    // A PalmDB the engine's MOBI path does not open.
    std::fs::write(docs.join("Topaz.azw").as_path(), palmdb(b"TPZ3TPZ3", 2)).unwrap();

    let found = scan_one(&docs, &cfg_out(&out));
    let mut titles: Vec<&str> = found.iter().map(|b| b.title.as_str()).collect();
    titles.sort();
    // The encrypted books list; the type-0 book does not.
    assert_eq!(titles, ["Old DRM", "Purchased"]);
    assert!(found.iter().all(|b| b.format == Format::Mobi));
}

#[test]
fn already_decrypted_books_are_marked_or_hidden() {
    let t = Tree::new("done");
    let items = t.dir("Items01");
    let out = t.dir("out");
    t.kfx(&items, "Fresh_B01MXXZOEW", true);
    t.kfx(&items, "Done_B078H4RWP7", true);
    std::fs::write(out.join("Done_B078H4RWP7.kfx-zip"), b"z").unwrap();
    // The MOBI output keeps the book's own name, so that is what marks it.
    t.mobi(&items, "Done Mobi.azw3", 2);
    std::fs::write(out.join("Done Mobi.azw3"), b"z").unwrap();

    let mut cfg = cfg_out(&out);
    let found = scan_one(&items, &cfg);
    assert_eq!(found.len(), 3);
    let mut done: Vec<&str> = found
        .iter()
        .filter(|b| b.done)
        .map(|b| b.title.as_str())
        .collect();
    done.sort();
    assert_eq!(done, ["Done", "Done Mobi"]);

    cfg.show_done = false;
    let found = scan_one(&items, &cfg);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "Fresh");
}

#[test]
fn done_is_judged_against_the_configured_output_folder() {
    // `Book::done` tracks `Config::out_dir`.
    let t = Tree::new("outdir");
    let items = t.dir("Items01");
    let out = t.dir("out");
    let elsewhere = t.dir("elsewhere");
    t.kfx(&items, "Book_B078H4RWP7", true);
    std::fs::write(out.join("Book_B078H4RWP7.kfx-zip"), b"z").unwrap();

    assert!(scan_one(&items, &cfg_out(&out))[0].done);
    assert!(!scan_one(&items, &cfg_out(&elsewhere))[0].done);
}

#[test]
fn each_format_toggle_removes_only_its_own_books() {
    let t = Tree::new("types");
    let dir = t.dir("books");
    let out = t.dir("out");
    t.kfx(&dir, "A_B000O76ON6", true);
    t.mobi(&dir, "B.azw3", 2);

    let mut cfg = cfg_out(&out);
    assert_eq!(scan_one(&dir, &cfg).len(), 2);

    cfg.types_mobi = false;
    let only_kfx = scan_one(&dir, &cfg);
    assert_eq!(only_kfx.len(), 1);
    assert_eq!(only_kfx[0].format, Format::Kfx);

    cfg.types_mobi = true;
    cfg.types_kfx = false;
    let only_mobi = scan_one(&dir, &cfg);
    assert_eq!(only_mobi.len(), 1);
    assert_eq!(only_mobi[0].format, Format::Mobi);
}

#[test]
fn the_scan_stops_at_one_level() {
    let t = Tree::new("depth");
    let dir = t.dir("books");
    let out = t.dir("out");
    t.kfx(&dir, "Top_B000O76ON6", true);
    // `updates/` stages partial downloads.
    let updates = t.dir("books/updates");
    t.kfx(&updates, "Staged_B01MXXZOEW", true);

    let found = scan_one(&dir, &cfg_out(&out));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "Top");
}

#[test]
fn roots_are_listed_in_the_order_they_were_given() {
    let t = Tree::new("order");
    let items = t.dir("Items01");
    let docs = t.dir("documents");
    let out = t.dir("out");
    t.kfx(&items, "Purchase_B000O76ON6", true);
    t.mobi(&docs, "Sideload.azw3", 2);

    let roots = [items, docs];
    let found = scan::scan_in(&roots, &cfg_out(&out), Path::new("/nonexistent"));
    let titles: Vec<&str> = found.iter().map(|b| b.title.as_str()).collect();
    // The order `Config::scan_roots` emits.
    assert_eq!(titles, ["Purchase", "Sideload"]);
}

#[test]
fn a_cover_is_taken_only_when_it_is_complete() {
    let t = Tree::new("cover");
    let items = t.dir("Items01");
    let out = t.dir("out");
    let thumbs = t.dir("thumbnails");
    t.kfx(&items, "Covered_B000O76ON6", true);
    t.kfx(&items, "Pending_B01MXXZOEW", true);
    t.kfx(&items, "Sideload", true); // no ASIN in the name at all
    std::fs::write(thumbs.join("thumbnail_B000O76ON6_EBOK_portrait.jpg"), b"j").unwrap();
    // A half-written JPEG.
    std::fs::write(
        thumbs.join("thumbnail_B01MXXZOEW_EBOK_portrait.jpg.tmp.partial"),
        b"p",
    )
    .unwrap();

    let found = scan::scan_in(&[items], &cfg_out(&out), &thumbs);
    let by = |t: &str| found.iter().find(|b| b.title == t).unwrap();
    assert!(by("Covered").cover_path.is_some());
    assert_eq!(by("Pending").cover_path, None);
    // No `Book::asin`: no cover key, and the whole stem is the title.
    assert_eq!(by("Sideload").asin, None);
    assert_eq!(by("Sideload").cover_path, None);
}

#[test]
fn an_absent_root_yields_nothing_rather_than_failing() {
    let t = Tree::new("absent");
    let out = t.dir("out");
    assert!(scan_one(&t.0.join("does-not-exist"), &cfg_out(&out)).is_empty());
}

#[test]
fn a_missing_engine_and_a_broken_one_are_told_apart() {
    let t = Tree::new("engine");
    let bin = t.0.join("bin");
    // Nothing installed at all.
    assert_eq!(engine::locate_in(&bin), Err(Missing::NotInstalled));

    // The directory exists and holds nothing that runs.
    std::fs::create_dir_all(&bin).unwrap();
    assert_eq!(engine::locate_in(&bin), Err(Missing::NoWorkingBuild));

    // The probe runs `test` and takes the exit code, not the name.
    std::fs::write(bin.join("kfxdedrmhf_c11"), b"not an elf").unwrap();
    assert_eq!(engine::locate_in(&bin), Err(Missing::NoWorkingBuild));
}
