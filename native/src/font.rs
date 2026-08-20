//! Which face draws which character.
//!
//! [`discover`] keeps every face on the device, [`PREFERRED`] first. A
//! `scan::Book::title` comes from a filename and carries any script, and the
//! chain is what keeps such a title off `.notdef`.
//!
//! [`FontChain::select`] picks per string, dropping to per character when no
//! one face covers the run. [`Script::of_language`] reorders the chain; no
//! caller here holds a language tag, leaving every candidate
//! [`Script::Unknown`] and the [`discover`] order standing.
//!
//! [`FontChain::glyph_source`] reads a fallback face on first miss. A loaded
//! face costs its file size resident, and a CJK face runs to tens of megabytes
//! against the device's 512 MB.
//!
//! [`covering_face`] and [`face_with`] take a coverage oracle, and run on a
//! host holding none of these faces.

use std::path::{Path, PathBuf};

use ab_glyph::{Font as _, FontVec};
use anyhow::{Result, anyhow};

/// The regional convention a run is set in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Script {
    /// No preference. [`visiting_order`] promotes nothing for this.
    #[default]
    Unknown,
    Japanese,
    SimplifiedChinese,
    TraditionalChinese,
}

impl Script {
    /// A BCP-47 tag as a preference, `_` or `-` separated, any case.
    pub fn of_language(tag: &str) -> Script {
        let mut subtags = tag.split(['-', '_']).map(str::trim);
        let primary = subtags.next().unwrap_or_default().to_ascii_lowercase();
        match primary.as_str() {
            // `jp` is a country code carried by some imported metadata.
            "ja" | "jp" => Script::Japanese,
            // CLDR resolves `yue` to `yue-Hant-HK`.
            "yue" => Script::TraditionalChinese,
            "zh" => {
                for subtag in subtags {
                    match subtag.to_ascii_lowercase().as_str() {
                        "hant" | "tw" | "hk" | "mo" => return Script::TraditionalChinese,
                        "hans" | "cn" | "sg" => return Script::SimplifiedChinese,
                        _ => {}
                    }
                }
                // CLDR resolves bare `zh` to `zh-Hans-CN`.
                Script::SimplifiedChinese
            }
            _ => Script::Unknown,
        }
    }
}

/// One face on the device, and the convention it sets.
pub struct Candidate {
    pub path: PathBuf,
    pub script: Script,
}

/// Directories [`discover`] reads.
///
/// Filenames vary across firmware generations. A missing directory costs one
/// failed `read_dir`.
const FONT_DIRS: &[&str] = &[
    // Holds the firmware set, including TBGothicMed, STHeitiMedium,
    // STHeitiTC and code2000.
    "/usr/java/lib/fonts",
    // Absent on current firmware.
    "/usr/share/fonts",
    // User-installed faces, last: a stray file cannot displace the UI font.
    "/mnt/us/fonts",
];

// `/chroot/usr/java/lib/fonts` mirrors the first entry and is left out.

/// Face families in preference order, matched case-insensitively against the
/// filename.
///
/// Ember is the Kindle's UI typeface. Bookerly is its reading face;
/// Baskerville and Caecilia follow. `code2000` is pan-Unicode with the widest
/// coverage and sorts last, leaving [`discover`] non-empty.
const PREFERRED: &[&str] = &[
    "ember",
    "bookerly",
    "baskerville",
    "caecilia",
    "helvetica",
    "code2000",
];

/// Distance from a plain upright; lower sorts earlier.
///
/// The tokens overlap on real filenames, and this order resolves them:
/// `Amazon-Ember-RegularItalic` carries "regular" and is italic;
/// `AmazonEmberBold-Regular` is the Bold family's regular cut;
/// `Amazon-Ember-Heavy` and `-Medium` carry no "bold" token and rank below
/// `-Regular`.
fn weight_rank(lower: &str) -> usize {
    if lower.contains("italic") || lower.contains("oblique") {
        return 3;
    }
    if ["bold", "heavy", "black", "light", "thin", "cond", "medium"]
        .iter()
        .any(|w| lower.contains(w))
    {
        return 2;
    }
    if lower.contains("regular") {
        return 0;
    }
    // No weight token, as in `code2000`. Ranks below an explicit "regular".
    1
}

/// [`PREFERRED`] position, then [`weight_rank`]. Lower sorts earlier.
fn rank(file_name: &str) -> (usize, usize) {
    let lower = file_name.to_ascii_lowercase();
    let family = PREFERRED
        .iter()
        .position(|p| lower.contains(p))
        .unwrap_or(PREFERRED.len());
    (family, weight_rank(&lower))
}

/// Every face on the device, [`rank`] order.
///
/// [`FontChain::load`] reads entry one and leaves the rest `Pending`.
///
/// [`rank`] orders [`PREFERRED`] ahead of everything else. A face outside
/// [`PREFERRED`] takes rank [`PREFERRED`]`.len()` and stays in the chain.
pub fn discover() -> Vec<Candidate> {
    let mut found: Vec<(usize, usize, PathBuf)> = Vec::new();
    for dir in FONT_DIRS {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !matches!(ext.as_str(), "ttf" | "otf" | "ttc") {
                continue;
            }
            let (family, styled) = rank(name);
            found.push((family, styled, path));
        }
    }
    found.sort();
    found
        .into_iter()
        .map(|(_, _, path)| Candidate {
            path,
            // No candidate claims a script; [`visiting_order`] promotes none.
            script: Script::Unknown,
        })
        .collect()
}

/// The face for a run. Made once per string, consulted per character, keeping
/// the metrics pass and the blit pass on one face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    /// One face covers the whole run.
    Whole(usize),
    /// No one face covers the run. Each character resolves separately, in the
    /// carried [`Script`] order.
    PerChar(Script),
}

/// `promoted` first, then the chain as declared.
///
/// `promoted` is a chain position. A firmware missing a face shortens the
/// chain.
pub fn visiting_order(faces: usize, promoted: Option<usize>) -> impl Iterator<Item = usize> {
    promoted
        .into_iter()
        .chain((0..faces).filter(move |face| Some(*face) != promoted))
}

/// First face in `order` that has every visible character of `text`.
///
/// `has_glyph(face, ch)` answers coverage, asked until a face misses. A face
/// no string needs goes unconsulted and unread.
pub fn covering_face<I, F>(text: &str, order: I, mut has_glyph: F) -> Option<usize>
where
    I: IntoIterator<Item = usize>,
    F: FnMut(usize, char) -> bool,
{
    order.into_iter().find(|&face| {
        text.chars()
            .filter(|c| !is_invisible(*c))
            .all(|c| has_glyph(face, c))
    })
}

/// First face in `order` that has `ch`, or `None` when none does.
pub fn face_with<I, F>(ch: char, order: I, mut has_glyph: F) -> Option<usize>
where
    I: IntoIterator<Item = usize>,
    F: FnMut(usize, char) -> bool,
{
    order.into_iter().find(|&face| has_glyph(face, ch))
}

/// Code points that carry no glyph: the C0/C1 controls, plus the zero-width
/// and formatting family — the BOM and zero-width spaces, bidi marks, the word
/// joiner and invisible operators, and the soft hyphen.
///
/// A face answers "no glyph" with `.notdef`, drawing a visible box. One of
/// these in a run also drops the run to [`Selection::PerChar`], since coverage
/// is decided per string.
///
/// A control is skipped, never interpreted. `\n` in particular is a layout
/// instruction that whoever owns the layout has to consume first — see
/// [`crate::ui::toast`], which lays a message out one row per line.
pub fn is_invisible(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{00AD}'                  // soft hyphen
            | '\u{200B}'..='\u{200F}'   // ZWSP, ZWNJ, ZWJ, LRM, RLM
            | '\u{2060}'..='\u{2064}'   // word joiner + invisible operators
            | '\u{FEFF}'                // BOM / zero-width no-break space
        )
}

/// An ordered set of faces: the first usable candidate, read up front, plus
/// the rest of the chain waiting on disk.
pub struct FontChain {
    primary: FontVec,
    primary_path: PathBuf,
    primary_script: Script,
    rest: Vec<Face>,
}

/// A fallback slot. [`FontChain::load`] drops candidates the firmware lacks,
/// leaving the chain at the files on the device. `path` outlives the read and
/// is what [`FontChain::paths`] reports.
struct Face {
    path: PathBuf,
    script: Script,
    state: State,
}

enum State {
    /// On disk, not parsed yet.
    Pending,
    Loaded(FontVec),
    /// Unreadable or unparseable, skipped from here on: one failed attempt
    /// per session.
    Absent,
}

impl FontChain {
    /// Take the `candidates` this firmware actually has and keep the first
    /// that parses as the primary; the rest become fallbacks, unread until a
    /// character misses.
    ///
    /// Fails only when none of them loads. A firmware that has moved or
    /// dropped one face is not a reason to refuse to start — the app draws
    /// with whatever it finds, and only an empty chain has nothing to say.
    pub fn load(candidates: &[Candidate]) -> Result<Self> {
        // Existence is settled here, parsing is not: a stat per candidate is
        // free, and it keeps `paths` an honest account of this device.
        let mut present = candidates
            .iter()
            .filter(|candidate| candidate.path.is_file());
        let mut primary = None;
        for candidate in present.by_ref() {
            if let Some(font) = read_face(&candidate.path) {
                primary = Some((font, candidate));
                break;
            }
        }
        let Some((primary, first)) = primary else {
            let tried: Vec<String> = candidates
                .iter()
                .map(|c| c.path.display().to_string())
                .collect();
            return Err(anyhow!("no usable font among {tried:?}"));
        };
        let rest = present
            .map(|candidate| Face {
                path: candidate.path.clone(),
                script: candidate.script,
                state: State::Pending,
            })
            .collect();
        Ok(Self {
            primary,
            primary_path: first.path.clone(),
            primary_script: first.script,
            rest,
        })
    }

    /// Faces in the chain, read or not. [`FontChain::load`] fails at zero.
    pub fn faces(&self) -> usize {
        1 + self.rest.len()
    }

    /// The chain on this device, primary first. Logged at startup; a moved or
    /// dropped face surfaces here ahead of a glyph that does not draw.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        std::iter::once(self.primary_path.as_path())
            .chain(self.rest.iter().map(|face| face.path.as_path()))
    }

    /// The face line metrics come from, holding every row to one height.
    pub fn primary(&self) -> &FontVec {
        &self.primary
    }

    /// Face for the whole of `text`, preferring the one that sets `script` —
    /// see [`covering_face`].
    pub fn select(&mut self, text: &str, script: Script) -> Selection {
        let order = visiting_order(self.faces(), self.promoted(script));
        match covering_face(text, order, |face, ch| {
            self.ensure(face).is_some_and(|font| has_glyph(font, ch))
        }) {
            Some(face) => Selection::Whole(face),
            None => Selection::PerChar(script),
        }
    }

    /// The face index and the face itself for `ch` under `selection`, or
    /// `None` when nothing in the chain has the character. The index is the
    /// glyph cache's key: two faces rasterize the same codepoint differently.
    pub fn glyph_source(&mut self, selection: Selection, ch: char) -> Option<(usize, &FontVec)> {
        let face = match selection {
            Selection::Whole(face) => face,
            Selection::PerChar(script) => {
                let order = visiting_order(self.faces(), self.promoted(script));
                face_with(ch, order, |face, c| {
                    self.ensure(face).is_some_and(|font| has_glyph(font, c))
                })?
            }
        };
        self.ensure(face).map(|font| (face, font))
    }

    /// Chain position of the face that sets `script`, if this device has it.
    ///
    /// [`Script::Unknown`] promotes nothing. The pan-Unicode catch-all sets it
    /// too, and a match there puts the chain's weakest face first.
    fn promoted(&self, script: Script) -> Option<usize> {
        if script == Script::Unknown {
            return None;
        }
        std::iter::once(self.primary_script)
            .chain(self.rest.iter().map(|face| face.script))
            .position(|candidate| candidate == script)
    }

    /// Face `index`, reading it from disk on first use.
    fn ensure(&mut self, index: usize) -> Option<&FontVec> {
        if index == 0 {
            return Some(&self.primary);
        }
        let face = self.rest.get_mut(index - 1)?;
        if matches!(face.state, State::Pending) {
            face.state = match read_face(&face.path) {
                Some(font) => State::Loaded(font),
                None => State::Absent,
            };
        }
        match &face.state {
            State::Loaded(font) => Some(font),
            State::Pending | State::Absent => None,
        }
    }
}

/// Whether `font` can draw `ch` at all. Glyph 0 is `.notdef`, which is what a
/// face hands back for a character it doesn't have, so a caller that skips this
/// check rasterizes tofu.
pub fn has_glyph(font: &FontVec, ch: char) -> bool {
    font.glyph_id(ch).0 != 0
}

/// Read and parse one candidate. `None` covers both a path this firmware
/// doesn't have and a file that won't parse: either way the answer is "skip
/// this face", not "fail" — the chain only has to keep one.
fn read_face(path: &Path) -> Option<FontVec> {
    let bytes = std::fs::read(path).ok()?;
    FontVec::try_from_vec(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Coverage oracle over repertoires spelled as strings: face 0 the
    /// Japanese face, face 1 the Simplified one, on down the chain.
    fn repertoires<'a>(faces: &'a [&'a str]) -> impl FnMut(usize, char) -> bool + 'a {
        move |face, ch| faces[face].contains(ch)
    }

    /// The chain tried in declared order — what a caller that knows nothing
    /// about the text's language gets.
    fn unhinted(faces: &[&str]) -> impl Iterator<Item = usize> {
        visiting_order(faces.len(), None)
    }

    #[test]
    fn a_run_one_face_covers_is_drawn_entirely_by_it() {
        // Every character is in face 0; the fallback goes unconsulted —
        // a Japanese title keeps Japanese shapes throughout.
        let faces = ["あいう漢字", "汉字"];
        let face = covering_face("漢字あい", unhinted(&faces), repertoires(&faces));
        assert_eq!(face, Some(0));
    }

    #[test]
    fn a_run_the_first_face_misses_moves_whole_to_the_next() {
        // 楼 is in both faces, 红 only in face 1. Selection is per string, so
        // 楼 is drawn by face 1, not by face 0.
        let faces = ["楼梦", "红楼梦魇"];
        assert_eq!(
            covering_face("红楼梦魇", unhinted(&faces), repertoires(&faces)),
            Some(1)
        );
    }

    #[test]
    fn a_run_no_single_face_covers_resolves_per_character() {
        let faces = ["あ", "汉"];
        assert_eq!(
            covering_face("あ汉", unhinted(&faces), repertoires(&faces)),
            None
        );
        assert_eq!(
            face_with('あ', unhinted(&faces), repertoires(&faces)),
            Some(0)
        );
        assert_eq!(
            face_with('汉', unhinted(&faces), repertoires(&faces)),
            Some(1)
        );
    }

    #[test]
    fn a_character_no_face_has_resolves_to_nothing() {
        let faces = ["あ", "汉"];
        assert_eq!(face_with('𐀀', unhinted(&faces), repertoires(&faces)), None);
    }

    #[test]
    fn invisible_characters_do_not_decide_the_face() {
        // A title carrying a stray BOM must not be pushed off the face that
        // covers its visible text — no face has a glyph for U+FEFF.
        let faces = ["あいう", "汉字"];
        assert_eq!(
            covering_face("あ\u{FEFF}い", unhinted(&faces), repertoires(&faces)),
            Some(0)
        );
        assert!(is_invisible('\u{FEFF}'));
        assert!(!is_invisible('あ'));
    }

    #[test]
    fn control_characters_never_reach_the_rasterizer() {
        // A banner message joins its clauses with `\n`, and no face has
        // U+000A: a newline reaching the renderer would draw the missing-glyph
        // box and, by missing everywhere, drop the rest of the line to
        // per-character resolution. The layout consumes it; the renderer skips
        // whatever is left, here and for any other control that rides in on
        // metadata.
        for c in ['\n', '\r', '\t', '\u{0}', '\u{7F}', '\u{85}'] {
            assert!(is_invisible(c), "{c:?} would draw as a box");
        }
        // Nothing in the chain has one, so it costs twice: the box is drawn,
        // and the miss pushes the rest of the run off its face.
        let faces = ["Synced 3", "汉字"];
        assert_eq!(face_with('\n', unhinted(&faces), repertoires(&faces)), None);
        assert_eq!(
            covering_face("Synced 3\nSynced 3", unhinted(&faces), repertoires(&faces)),
            Some(0)
        );
    }

    #[test]
    fn an_empty_run_selects_the_first_face() {
        let faces = ["あ"];
        assert_eq!(
            covering_face("", unhinted(&faces), repertoires(&faces)),
            Some(0)
        );
    }

    #[test]
    fn a_hint_moves_its_face_to_the_front_and_keeps_the_rest_in_order() {
        assert_eq!(visiting_order(4, Some(2)).collect::<Vec<_>>(), [2, 0, 1, 3]);
        assert_eq!(visiting_order(4, None).collect::<Vec<_>>(), [0, 1, 2, 3]);
        assert_eq!(visiting_order(4, Some(0)).collect::<Vec<_>>(), [0, 1, 2, 3]);
    }

    #[test]
    fn a_hint_wins_over_a_face_that_would_also_have_covered_the_run() {
        // The whole point of the hint: face 0 (Japanese) has every character
        // of this Traditional title; coverage alone leaves it there
        // in Japanese shapes. The Traditional face is face 2.
        let faces = ["粵語語法講義", "粤语语法讲义", "粵語語法講義"];
        assert_eq!(
            covering_face("粵語語法講義", visiting_order(3, None), repertoires(&faces)),
            Some(0)
        );
        assert_eq!(
            covering_face(
                "粵語語法講義",
                visiting_order(3, Some(2)),
                repertoires(&faces)
            ),
            Some(2)
        );
    }

    #[test]
    fn a_hinted_face_that_misses_still_loses_to_coverage() {
        // A wrong language tag costs regional shapes, never glyphs.
        let faces = ["紅樓夢魘", "红楼梦魇"];
        assert_eq!(
            covering_face("红楼梦魇", visiting_order(2, Some(0)), repertoires(&faces)),
            Some(1)
        );
    }

    #[test]
    fn language_tags_name_the_convention_they_are_set_in() {
        assert_eq!(Script::of_language("ja"), Script::Japanese);
        // A country code where a language belongs — real imported metadata.
        assert_eq!(Script::of_language("jp"), Script::Japanese);
        assert_eq!(Script::of_language("zh-Hant"), Script::TraditionalChinese);
        assert_eq!(Script::of_language("zh_TW"), Script::TraditionalChinese);
        assert_eq!(Script::of_language("ZH-HK"), Script::TraditionalChinese);
        assert_eq!(Script::of_language("yue"), Script::TraditionalChinese);
        assert_eq!(Script::of_language("zh-Hans"), Script::SimplifiedChinese);
        assert_eq!(Script::of_language("zh-CN"), Script::SimplifiedChinese);
        // Bare `zh` is Simplified, per CLDR's likely subtags.
        assert_eq!(Script::of_language("zh"), Script::SimplifiedChinese);
        // No chain entry sets these; they express no preference.
        assert_eq!(Script::of_language("en"), Script::Unknown);
        assert_eq!(Script::of_language("ko"), Script::Unknown);
        assert_eq!(Script::of_language(""), Script::Unknown);
    }

    #[test]
    fn the_ui_font_wins_on_a_real_device() {
        // Every Latin face on a Colorsoft (firmware 5.18), verbatim. Three
        // carry no "bold" token and rank below Regular.
        let mut faces = [
            "Amazon-Ember-Bold.ttf",
            "Amazon-Ember-BoldItalic.ttf",
            "Amazon-Ember-Heavy.ttf",
            "Amazon-Ember-HeavyItalic.ttf",
            "Amazon-Ember-Medium.ttf",
            "Amazon-Ember-MediumItalic.ttf",
            "Amazon-Ember-Regular.ttf",
            "Amazon-Ember-RegularItalic.ttf",
            "AmazonEmberBold-Bold.ttf",
            "AmazonEmberBold-Regular.ttf",
            "Baskerville-Regular.ttf",
            "Bookerly-Regular.ttf",
            "BookerlyDisplay-Regular.ttf",
            "Caecilia_LT_65_Medium.ttf",
            "Futura-Medium.ttf",
            "Helvetica_LT_65_Medium.ttf",
            "KindleBlackboxRegular.ttf",
            "Palatino-Regular.ttf",
            "code2000.ttf",
        ];
        faces.sort_by_key(|f| (rank(f), *f));

        assert_eq!(
            faces[0], "Amazon-Ember-Regular.ttf",
            "the device's UI typeface, upright and regular, must draw the UI"
        );
        // The traps a plain token search falls into: a bold family whose cut
        // is named "Regular", and two weights with no bold-ish token in the
        // name at all.
        for trap in [
            "AmazonEmberBold-Regular.ttf",
            "Amazon-Ember-Heavy.ttf",
            "Amazon-Ember-Medium.ttf",
            "Amazon-Ember-RegularItalic.ttf",
        ] {
            assert!(
                rank("Amazon-Ember-Regular.ttf") < rank(trap),
                "{trap} should rank below the plain regular"
            );
        }
        // `code2000` sorts below every other [`PREFERRED`] family and above
        // the faces outside it (Blackbox, Futura, the Noto packs).
        for named in ["Bookerly-Regular.ttf", "Baskerville-Regular.ttf"] {
            assert!(rank(named) < rank("code2000.ttf"));
        }
        assert!(rank("code2000.ttf") < rank("KindleBlackboxRegular.ttf"));
    }

    #[test]
    fn language_classification_still_works_even_though_nothing_uses_it() {
        // No caller holds a language tag. `text.rs` and `grid.rs` read
        // `Script` in their signatures.
        assert_eq!(Script::of_language("en"), Script::Unknown);
        assert_eq!(Script::of_language(""), Script::Unknown);
    }

    #[test]
    fn a_chain_with_no_readable_candidate_fails_to_load() {
        // The one fatal case. A single missing candidate is skipped, and that
        // path needs a real font file.
        let nowhere = [Candidate {
            path: PathBuf::from("/nonexistent/font.ttf"),
            script: Script::Unknown,
        }];
        let Err(err) = FontChain::load(&nowhere) else {
            panic!("a chain over a path that isn't there has nothing to draw with");
        };
        assert!(err.to_string().contains("no usable font"));
        assert!(FontChain::load(&[]).is_err());
    }
}
