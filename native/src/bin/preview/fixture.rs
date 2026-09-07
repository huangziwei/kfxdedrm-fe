//! The books, folders and settings the shots are drawn against.
//!
//! Titles run to the lengths a real library reaches, and carry the scripts the
//! font chain has to fall back for — a shot is worth taking mostly to see what
//! the layout does with the long ones.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use kfxdedrm_fe_native::config::{self, Config};
use kfxdedrm_fe_native::engine::Format;
use kfxdedrm_fe_native::scan::{Book, Candidate};
use kfxdedrm_fe_native::ui::configmenu::{AddOn, App, Installed};

/// Titles in the shape `scan::title_from_stem` leaves them, longest last: the
/// grid's label is where a cell runs out of room.
const TITLES: &[&str] = &[
    "Dune",
    "The Dispossessed",
    "Piranesi",
    "A Memory Called Empire",
    "スカイ・クロラ",
    "三体",
    "The Left Hand of Darkness",
    "Gödel, Escher, Bach: An Eternal Golden Braid",
    "Записки из подполья",
    "The Master and Margarita",
    "La Sombra del Viento",
    "Never Let Me Go",
];

/// `count` books, `done` on every one or none. The formats alternate so both
/// `engine::output_path` names appear.
pub fn books(count: usize, done: bool) -> Vec<Book> {
    (0..count)
        .map(|i| {
            let title = TITLES[i % TITLES.len()];
            let format = match i % 3 {
                0 => Format::Mobi,
                _ => Format::Kfx,
            };
            let stem = format!("{}_B0{:06}", title.replace(' ', "_"), i);
            Book {
                path: PathBuf::from(config::ITEMS01_DIR).join(format!("{stem}.kfx")),
                format,
                title: title.to_string(),
                asin: Some(format!("B0{i:06}")),
                cover_path: None,
                size: 2_400_000 + (i as u64 * 517_000),
                // Newest first, an hour apart, so the order is the scan's.
                mtime: SystemTime::UNIX_EPOCH
                    + Duration::from_secs(1_780_000_000 - i as u64 * 3600),
                done,
            }
        })
        .collect()
}

/// The folder chips Settings offers.
pub fn folders() -> Vec<Candidate> {
    vec![
        Candidate {
            dir: PathBuf::from(config::ITEMS01_DIR),
            books: 12,
        },
        Candidate {
            dir: PathBuf::from(config::DOCUMENTS_DIR),
            books: 3,
        },
        Candidate {
            dir: PathBuf::from(config::DOCUMENTS_DIR).join("Books"),
            books: 41,
        },
    ]
}

/// Both converter chips on, which is the page with nothing greyed out.
pub fn config() -> Config {
    Config {
        scan_dirs: vec![
            PathBuf::from(config::ITEMS01_DIR),
            PathBuf::from(config::DOCUMENTS_DIR),
        ],
        out_dir: PathBuf::from(config::DEFAULT_OUT_DIR),
        types_kfx: true,
        types_mobi: true,
        pack_kfx: true,
        convert_epub: true,
        show_done: true,
    }
}

/// What the status line and the `Get` row report. `present` covers both
/// add-ons: false is the install a first run finds.
pub fn installed(present: bool) -> Installed {
    Installed {
        engine: AddOn {
            present,
            tag: present.then(|| "v10.0.3".to_string()),
        },
        bokai: AddOn {
            present,
            tag: present.then(|| "v0.1.9".to_string()),
        },
        app: App {
            version: "0.5.2".to_string(),
            staged: false,
        },
    }
}
