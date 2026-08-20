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
use kfxdedrm_fe_native::convert::{self, Targets};
use kfxdedrm_fe_native::engine::{self, Format, Missing};
use kfxdedrm_fe_native::scan::{self, Book, Candidate};

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

/// A `Config` listing every format and selecting no folder.
///
/// Each test passes its own roots and out folder to [`scan::scan_in`], and
/// `Config::default`'s selection is an on-device path that no [`Tree`] holds —
/// `scan::candidates_in` would carry it onto every list here.
fn cfg() -> Config {
    Config {
        scan_dirs: Vec::new(),
        ..Config::default()
    }
}

/// [`scan::scan_in`] over one root, no thumbnail cache and no conversions.
fn scan_one(root: &Path, out: &Path, cfg: &Config) -> Vec<Book> {
    scan::scan_in(
        &[root.to_path_buf()],
        cfg,
        Targets::default(),
        out,
        Path::new("/nonexistent"),
    )
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

    let found = scan_one(&items, &out, &cfg());
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

    let found = scan_one(&docs, &out, &cfg());
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

    let mut cfg = cfg();
    let found = scan_one(&items, &out, &cfg);
    assert_eq!(found.len(), 3);
    let mut done: Vec<&str> = found
        .iter()
        .filter(|b| b.done)
        .map(|b| b.title.as_str())
        .collect();
    done.sort();
    assert_eq!(done, ["Done", "Done Mobi"]);

    cfg.show_done = false;
    let found = scan_one(&items, &out, &cfg);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "Fresh");
}

/// `convert::Targets` rides `Book::done`: a decrypt whose conversions have
/// not landed is not finished, and the book stays listed for another run.
#[test]
fn a_book_missing_a_conversion_is_not_done_yet() {
    let t = Tree::new("converted");
    let items = t.dir("Items01");
    let out = t.dir("out");
    t.kfx(&items, "Book_B078H4RWP7", true);
    std::fs::write(out.join("Book_B078H4RWP7.kfx-zip"), b"z").unwrap();

    let cfg = cfg();
    let targets = Targets {
        kfx: true,
        epub: true,
    };
    let scan = |targets| {
        scan::scan_in(
            std::slice::from_ref(&items),
            &cfg,
            targets,
            &out,
            Path::new("/nonexistent"),
        )
    };

    // The engine's own output alone is enough only when nothing else is asked.
    assert!(scan(Targets::default())[0].done);
    assert!(!scan(targets)[0].done);

    std::fs::write(out.join("Book_B078H4RWP7.kfx"), b"k").unwrap();
    assert!(!scan(targets)[0].done);

    std::fs::write(out.join("Book_B078H4RWP7.epub"), b"e").unwrap();
    assert!(scan(targets)[0].done);
}

#[test]
fn done_is_judged_against_the_output_folder_it_is_given() {
    // `Book::done` tracks `scan::scan_in`'s out folder, which on the device is
    // `config::OUT_DIR`.
    let t = Tree::new("outdir");
    let items = t.dir("Items01");
    let out = t.dir("out");
    let elsewhere = t.dir("elsewhere");
    t.kfx(&items, "Book_B078H4RWP7", true);
    std::fs::write(out.join("Book_B078H4RWP7.kfx-zip"), b"z").unwrap();

    assert!(scan_one(&items, &out, &cfg())[0].done);
    assert!(!scan_one(&items, &elsewhere, &cfg())[0].done);
}

#[test]
fn each_format_toggle_removes_only_its_own_books() {
    let t = Tree::new("types");
    let dir = t.dir("books");
    let out = t.dir("out");
    t.kfx(&dir, "A_B000O76ON6", true);
    t.mobi(&dir, "B.azw3", 2);

    let mut cfg = cfg();
    assert_eq!(scan_one(&dir, &out, &cfg).len(), 2);

    cfg.types_mobi = false;
    let only_kfx = scan_one(&dir, &out, &cfg);
    assert_eq!(only_kfx.len(), 1);
    assert_eq!(only_kfx[0].format, Format::Kfx);

    cfg.types_mobi = true;
    cfg.types_kfx = false;
    let only_mobi = scan_one(&dir, &out, &cfg);
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

    let found = scan_one(&dir, &out, &cfg());
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
    let found = scan::scan_in(
        &roots,
        &cfg(),
        Targets::default(),
        &out,
        Path::new("/nonexistent"),
    );
    let titles: Vec<&str> = found.iter().map(|b| b.title.as_str()).collect();
    // The order `Config::scan_dirs` names them in.
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

    let found = scan::scan_in(&[items], &cfg(), Targets::default(), &out, &thumbs);
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
    assert!(scan_one(&t.0.join("does-not-exist"), &out, &cfg()).is_empty());
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

/// `convert::locate_at` takes the `--version` exit code, the way
/// `engine::locate_in` takes `test`'s: a build for the wrong ABI is on disk
/// and does not run.
#[test]
fn the_converter_has_to_run_before_it_counts_as_installed() {
    let t = Tree::new("converter");
    let bin = t.dir("bin");
    let exe = bin.join("bokai");

    // Nothing there.
    assert_eq!(convert::locate_at(&exe), None);

    // There, and not executable.
    std::fs::write(&exe, b"not an elf").unwrap();
    assert_eq!(convert::locate_at(&exe), None);

    // A stand-in that exits 0. The probe never reads what it prints.
    std::fs::write(&exe, "#!/bin/sh\necho bokai 0.0.0\n").unwrap();
    make_executable(&exe);
    assert!(convert::locate_at(&exe).is_some_and(|c| c.exe() == exe));

    // Installed, and failing its own probe.
    std::fs::write(&exe, "#!/bin/sh\nexit 1\n").unwrap();
    make_executable(&exe);
    assert_eq!(convert::locate_at(&exe), None);
}

/// `chmod +x`, for the shell stand-ins above.
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
}

/// The rule `scan::candidates` offers a folder by: it holds a DRM'd book, at
/// any depth the walk reaches.
#[test]
fn a_folder_holding_a_drmd_book_is_offered_and_an_empty_one_is_not() {
    let t = Tree::new("candidates");
    let documents = t.dir("documents");
    let downloads = t.dir("documents/Downloads");
    let items01 = t.dir("documents/Downloads/Items01");
    let sidle = t.dir("documents/Sidle");
    // A folder with nothing in it, and one with a DRM-free book.
    t.dir("documents/Empty");
    let free = t.dir("documents/Free");
    t.mobi(&free, "Sideload.azw3", 0);

    t.kfx(&documents, "Old_B000O76ON6", true);
    t.kfx(&items01, "New_B01MXXZOEW", true);
    t.kfx(&items01, "Newer_B078H4RWP7", true);
    t.kfx(&sidle, "Mine_B00XST7S8C", true);

    let found = scan::candidates_in(&documents, &cfg());
    let named: Vec<(String, usize)> = found.iter().map(|c| (leaf(&c.dir), c.books)).collect();
    // The root itself, then its subfolders in name order — `Downloads` before
    // `Items01` under it. `Empty` and `Free` hold no DRM'd book and are gone.
    assert_eq!(
        named,
        [
            ("documents".to_string(), 1),
            ("Items01".to_string(), 2),
            ("Sidle".to_string(), 1),
        ]
    );
    // `Downloads` itself holds no book, so it is not offered even though the
    // walk went through it to reach `Items01`.
    assert!(!found.iter().any(|c| c.dir == downloads));
}

/// A `.sdr` sits beside every KFX book, so a library's worth of them is most
/// of what the documents folder holds — and none is a folder anyone scans.
#[test]
fn sidecar_folders_are_never_offered() {
    let t = Tree::new("sidecars");
    let documents = t.dir("documents");
    t.kfx(&documents, "Book_B000O76ON6", true);
    // The voucher `Tree::kfx` wrote lives in `Book_B000O76ON6.sdr/assets/`,
    // which is a real directory two levels down.
    let found = scan::candidates_in(&documents, &cfg());
    assert!(
        found
            .iter()
            .all(|c| !c.dir.to_string_lossy().contains(".sdr")),
        "{found:?}"
    );
}

/// A folder that is selected keeps its chip at zero books, or switching it off
/// would mean tapping a chip that is no longer drawn.
#[test]
fn a_selected_folder_stays_on_the_list_when_it_empties() {
    let t = Tree::new("selected");
    let documents = t.dir("documents");
    let empty = t.dir("documents/Empty");

    let mut cfg = cfg();
    assert!(scan::candidates_in(&documents, &cfg).is_empty());

    cfg.scan_dirs = vec![empty.clone()];
    let found = scan::candidates_in(&documents, &cfg);
    assert_eq!(
        found,
        [Candidate {
            dir: empty,
            books: 0
        }]
    );
}

/// The counts follow the format toggles, so a chip never promises books the
/// grid would not list.
#[test]
fn the_count_is_what_the_grid_would_show_there() {
    let t = Tree::new("counts");
    let documents = t.dir("documents");
    t.kfx(&documents, "Kfx_B000O76ON6", true);
    t.mobi(&documents, "Mobi.azw3", 2);

    let mut cfg = cfg();
    assert_eq!(scan::candidates_in(&documents, &cfg)[0].books, 2);

    cfg.types_mobi = false;
    assert_eq!(scan::candidates_in(&documents, &cfg)[0].books, 1);

    cfg.types_kfx = false;
    assert!(scan::candidates_in(&documents, &cfg).is_empty());
}

/// A book already decrypted still counts: a folder whose chip vanished once
/// its books were done could not be switched off again.
#[test]
fn a_finished_book_still_counts_towards_its_folder() {
    let t = Tree::new("finished");
    let documents = t.dir("documents");
    let out = t.dir("out");
    t.kfx(&documents, "Done_B078H4RWP7", true);
    std::fs::write(out.join("Done_B078H4RWP7.kfx-zip"), b"z").unwrap();

    let mut cfg = cfg();
    cfg.show_done = false;
    assert!(scan_one(&documents, &out, &cfg).is_empty());
    assert_eq!(scan::candidates_in(&documents, &cfg)[0].books, 1);
}

/// Last path component, for naming a folder in an assertion.
fn leaf(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}
