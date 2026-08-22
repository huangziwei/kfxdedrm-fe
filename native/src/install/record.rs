//! Which release of each add-on is installed, as `key = value` lines at
//! [`PATH`]. `koplugin/kfxdedrm.koplugin/lib/install.lua` renders the same
//! bytes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Beside `config.txt`, under this app's extension folder.
pub const PATH: &str = "/mnt/us/extensions/kfxdedrm-fe/installs.txt";

pub fn path() -> PathBuf {
    PathBuf::from(PATH)
}

/// `key = value` lines, one per `crate::install::Source`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Record {
    tags: BTreeMap<String, String>,
}

impl Record {
    /// [`Record::parse`] of `path`, or an empty one.
    pub fn load(path: &Path) -> Record {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(_) => Record::default(),
        }
    }

    /// [`Record::render`] to `path`, creating its parent.
    pub fn store(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.render())
    }

    /// `key = value` lines; blank lines, `#` comments and lines without `=`
    /// are skipped, and an empty value is no record at all.
    pub fn parse(text: &str) -> Record {
        let mut tags = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let (key, value) = (key.trim(), value.trim());
                if !key.is_empty() && !value.is_empty() {
                    tags.insert(key.to_string(), value.to_string());
                }
            }
        }
        Record { tags }
    }

    pub fn render(&self) -> String {
        let mut out = String::from(
            "\
# Which release of each add-on is installed. Both frontends write this file,
# the standalone kfxdedrm-fe app and the KOReader plugin, so neither fetches
# what the other already has. Delete a line to fetch that one again.
",
        );
        for (key, tag) in &self.tags {
            out.push_str(&format!("{key} = {tag}\n"));
        }
        out
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.tags.get(key).map(String::as_str)
    }

    pub fn set(&mut self, key: &str, tag: &str) {
        self.tags.insert(key.to_string(), tag.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file the plugin reads, and the one it writes.
    #[test]
    fn the_format_is_the_one_both_frontends_agree_on() {
        let mut r = Record::default();
        r.set("bokai", "v0.1.3");
        r.set("engine", "v10.0.30");
        // Sorted by key, one `key = value` line each, under a comment header:
        // `koplugin/spec/fixtures/installs.txt` is the same bytes.
        let rendered = r.render();
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines[lines.len() - 2], "bokai = v0.1.3");
        assert_eq!(lines[lines.len() - 1], "engine = v10.0.30");
        assert!(lines[..lines.len() - 2].iter().all(|l| l.starts_with('#')));
    }

    #[test]
    fn round_trips_through_the_file_format() {
        let mut r = Record::default();
        r.set("engine", "v10.0.30");
        r.set("bokai", "v0.1.3");
        assert_eq!(Record::parse(&r.render()), r);
        assert_eq!(r.get("engine"), Some("v10.0.30"));
        assert_eq!(r.get("nothing"), None);
    }

    #[test]
    fn an_empty_record_is_a_file_of_comments() {
        let empty = Record::default();
        assert_eq!(Record::parse(&empty.render()), empty);
        assert!(empty.render().lines().all(|l| l.starts_with('#')));
        assert_eq!(empty.get("engine"), None);
    }

    #[test]
    fn a_hand_edited_file_costs_only_the_lines_that_are_wrong() {
        let r = Record::parse(
            "\
             # a comment\n\
             \n\
             engine = v10.0.30\n\
             bokai =\n\
             nonsense\n\
             = orphan\n",
        );
        assert_eq!(r.get("engine"), Some("v10.0.30"));
        // An empty value is no record.
        assert_eq!(r.get("bokai"), None);
        assert_eq!(r, Record::parse("engine=v10.0.30"));
    }

    #[test]
    fn a_file_that_is_not_there_reads_as_nothing_installed() {
        assert_eq!(
            Record::load(Path::new("/nonexistent/kfxdedrm-fe/installs.txt")),
            Record::default()
        );
    }
}
