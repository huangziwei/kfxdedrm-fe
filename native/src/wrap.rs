//! Text wrapping over a width function.
//!
//! `ui::text::TextRenderer` supplies a font-backed `measure`; the tests here
//! supply synthetic widths.

/// Word-wrap `text` to fit `max_width` per line.
///
/// Whitespace breaks a Latin title. A CJK title and a single word wider than
/// `max_width` break per character.
///
/// `measure` is the per-substring width function.
pub fn wrap_to_width<F>(text: &str, max_width: u32, mut measure: F) -> Vec<String>
where
    F: FnMut(&str) -> u32,
{
    let space_w = measure(" ");
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_w = 0u32;
    let mut buf = [0u8; 4];

    for token in text.split_whitespace() {
        let token_w = measure(token);
        if token_w > max_width {
            // Token alone overflows a line — break at char boundaries.
            // CJK path (whole string is one token) and Latin edge
            // cases (URLs etc).
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current_w = 0;
            }
            for ch in token.chars() {
                let ch_str = ch.encode_utf8(&mut buf);
                let ch_w = measure(ch_str);
                if current_w + ch_w > max_width && !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                    current_w = 0;
                }
                current.push(ch);
                current_w += ch_w;
            }
            continue;
        }

        // Token fits in a line by itself. Append (with leading space)
        // or start a new line.
        let prefix_w = if current.is_empty() { 0 } else { space_w };
        if current_w + prefix_w + token_w > max_width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current_w = 0;
        }
        if !current.is_empty() {
            current.push(' ');
            current_w += space_w;
        }
        current.push_str(token);
        current_w += token_w;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Word-wrap `text` to `max_width`, then clamp to at most `max_lines`
/// lines — appending `…` to the last kept line whenever content was
/// dropped. The ellipsis is fitted by trimming trailing chars until
/// `"<line>…"` measures within `max_width`; the truncated line never
/// overflows the box (in the degenerate case the line collapses to just
/// `…`).
///
/// Same `measure` contract as [`wrap_to_width`]. Extracted from the cover
/// placeholder renderer so the diagnostics panel can clamp long error
/// strings the same way — one tested path for both.
pub fn wrap_and_clamp<F>(
    text: &str,
    max_width: u32,
    max_lines: usize,
    mut measure: F,
) -> Vec<String>
where
    F: FnMut(&str) -> u32,
{
    let mut lines = wrap_to_width(text, max_width, &mut measure);
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            // Trim trailing chars until "<last>…" fits the width, then
            // assign the ellipsized form back.
            let mut candidate = format!("{last}…");
            while !last.is_empty() && measure(&candidate) > max_width {
                last.pop();
                candidate = format!("{last}…");
            }
            *last = candidate;
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed-width "font" — every char is 10px, space is 10px. Lets us
    /// reason about wrap purely arithmetically.
    fn fixed(s: &str) -> u32 {
        s.chars().count() as u32 * 10
    }

    #[test]
    fn wraps_latin_at_word_boundaries() {
        // 100px max = 10 chars. "hello world" (11 chars) wraps after
        // "hello": the full string measures 110px against 100.
        let lines = wrap_to_width("hello world", 100, fixed);
        assert_eq!(lines, vec!["hello".to_string(), "world".to_string()]);
    }

    #[test]
    fn fits_single_line_when_under_max() {
        let lines = wrap_to_width("short", 100, fixed);
        assert_eq!(lines, vec!["short".to_string()]);
    }

    #[test]
    fn wraps_cjk_at_char_boundaries() {
        // 50px = 5 chars. "あいうえおかきくけこ" (10 chars) wraps to two
        // lines of 5. No whitespace → whole thing is one "token" →
        // char-level path.
        let lines = wrap_to_width("あいうえおかきくけこ", 50, fixed);
        assert_eq!(
            lines,
            vec!["あいうえお".to_string(), "かきくけこ".to_string()],
        );
    }

    #[test]
    fn empty_text_returns_no_lines() {
        let lines = wrap_to_width("", 100, fixed);
        assert!(lines.is_empty());
    }

    #[test]
    fn long_word_breaks_at_char_when_too_wide() {
        // 30px = 3 chars. "supercalifragilistic" (20 chars) wraps as
        // 3-char chunks. 20/3 = 6 full + 1 partial = 7 lines.
        let lines = wrap_to_width("supercalifragilistic", 30, fixed);
        assert_eq!(lines.len(), 7);
        assert_eq!(lines[0], "sup");
        assert_eq!(lines.last().unwrap(), "ic");
    }

    #[test]
    fn no_line_exceeds_max_in_mixed_text() {
        // Property check: every line ≤ max chars regardless of input
        // mix.
        let lines = wrap_to_width("a bb cccc ddddd", 30, fixed);
        assert!(lines.iter().all(|l| l.chars().count() <= 3));
    }

    #[test]
    fn clamp_keeps_all_lines_when_within_max() {
        // "hello world" wraps to 2 lines at 100px; max_lines 3 → returned
        // unchanged, no ellipsis.
        let lines = wrap_and_clamp("hello world", 100, 3, fixed);
        assert_eq!(lines, vec!["hello".to_string(), "world".to_string()]);
    }

    #[test]
    fn clamp_truncates_and_ellipsizes_last_line() {
        // 30px = 3 chars. "aaa bbb ccc ddd" wraps to 4 lines; clamp to 2.
        // The 2nd kept line "bbb" loses a char to make room for the
        // ellipsis (which itself measures 10px): "bb…" = 30px ≤ 30.
        let lines = wrap_and_clamp("aaa bbb ccc ddd", 30, 2, fixed);
        assert_eq!(lines, vec!["aaa".to_string(), "bb…".to_string()]);
    }

    #[test]
    fn clamp_to_one_line_ellipsizes() {
        let lines = wrap_and_clamp("aaa bbb ccc", 30, 1, fixed);
        assert_eq!(lines, vec!["aa…".to_string()]);
    }
}
