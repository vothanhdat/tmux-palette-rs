//! Display-width helpers — port of `src/text.ts`.
//!
//! tmux popups render in a monospaced grid, so the renderer needs to know how
//! many columns a string occupies *after* ANSI escapes are stripped and wide
//! (CJK / emoji) glyphs are counted as two cells.

const WIDE_RANGES: &[(u32, u32)] = &[
    (0x1100, 0x115f),
    (0x2329, 0x232a),
    (0x2e80, 0xa4cf),
    (0xac00, 0xd7a3),
    (0xf000, 0xf8ff),
    (0xfe10, 0xfe19),
    (0xfe30, 0xfe6f),
    (0xff00, 0xff60),
    (0xffe0, 0xffe6),
    (0x1f300, 0x1faff),
];

/// Remove SGR escape sequences (`\x1b[…m`), matching the original
/// `/\x1b\[[0-9;]*m/g` replace. Other escape sequences are left untouched.
pub fn strip(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\u{1b}' && i + 1 < chars.len() && chars[i + 1] == '[' {
            let mut j = i + 2;
            while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == ';') {
                j += 1;
            }
            if j < chars.len() && chars[j] == 'm' {
                i = j + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn is_wide(code: u32) -> bool {
    WIDE_RANGES.iter().any(|&(lo, hi)| code >= lo && code <= hi)
}

fn is_non_display(code: u32) -> bool {
    code == 0 || code < 32 || (0x7f..0xa0).contains(&code)
}

pub fn char_width(c: char) -> i64 {
    let code = c as u32;
    if is_non_display(code) {
        return 0;
    }
    if is_wide(code) {
        2
    } else {
        1
    }
}

pub fn display_width(s: &str) -> i64 {
    strip(s).chars().map(char_width).sum()
}

/// Pad with spaces to `width`, or truncate with a trailing `…`. When the string
/// already fits, ANSI styling is preserved; when it must be cut, styling is
/// stripped (matching the original — the cut path operates on plain text).
pub fn truncate(s: &str, width: i64) -> String {
    let current = display_width(s);
    if current <= width {
        let pad = (width - current).max(0) as usize;
        return format!("{}{}", s, " ".repeat(pad));
    }
    let plain = strip(s);
    let mut result = String::new();
    let mut used: i64 = 0;
    for c in plain.chars() {
        let next = used + char_width(c);
        if next >= width {
            break;
        }
        result.push(c);
        used = next;
    }
    let pad = (width - used - 1).max(0) as usize;
    format!("{}…{}", result, " ".repeat(pad))
}

/// Initials of a multi-word title (`Split Horizontal` → `sh`), used as an
/// invisible searchable alias. Single-word titles return `None`.
pub fn auto_alias(title: &str) -> Option<String> {
    let words: Vec<&str> = title
        .split_whitespace()
        .filter(|w| w.chars().next().is_some_and(|c| c.is_ascii_alphabetic()))
        .collect();
    if words.len() < 2 {
        return None;
    }
    Some(
        words
            .iter()
            .map(|w| w.chars().next().unwrap().to_ascii_lowercase())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_ascii_and_wide_glyphs() {
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width('😀'), 2);
        assert_eq!(display_width("a😀b"), 4);
    }

    #[test]
    fn ignores_ansi_color_escapes() {
        assert_eq!(display_width("\x1b[31mred\x1b[0m"), 3);
    }

    #[test]
    fn pads_short_strings() {
        assert_eq!(truncate("tmux", 6), "tmux  ");
    }

    #[test]
    fn truncates_long_strings_with_ellipsis() {
        assert_eq!(truncate("tmux-palette", 6), "tmux-…");
    }

    #[test]
    fn does_not_split_a_wide_glyph() {
        assert_eq!(truncate("ab😀cd", 5), "ab😀…");
    }

    #[test]
    fn builds_initials_from_multi_word_titles() {
        assert_eq!(auto_alias("Split Horizontal").as_deref(), Some("sh"));
    }

    #[test]
    fn ignores_single_word_titles() {
        assert_eq!(auto_alias("Detach"), None);
    }
}
