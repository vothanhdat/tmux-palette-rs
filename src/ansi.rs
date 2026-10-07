//! Styled screen text: parse `tmux capture-pane -e` output into cells and draw
//! them back at a given width.
//!
//! The capture can't be passed through as-is. Its SGR state carries from one
//! line to the next (a line opens with whatever the previous one left set), so
//! a line shown without the ones above it would lose its colors; it holds
//! sequences a popup shouldn't forward (OSC 8 hyperlinks, colon-form underline
//! styles the renderer can't measure); and `text::clip` strips the styling off
//! any line it has to cut. So every cell gets its fully resolved style, and rows
//! are re-emitted from those.

use crate::text::char_width;

/// SGR attributes tracked per cell, by their "on" parameter: bold, dim, italic,
/// underline, blink, reverse, hidden, strikethrough, overline.
const ATTRS: [u16; 9] = [1, 2, 3, 4, 5, 7, 8, 9, 53];

/// A cell's style. `None` colors are the pane's defaults, which `render_row`
/// draws in the caller's colors.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Style {
    /// SGR parameters selecting the foreground: `31`, `38;5;208`, `38;2;r;g;b`.
    fg: Option<String>,
    /// The same for the background: `44`, `48;5;17`, `48;2;r;g;b`.
    bg: Option<String>,
    /// Bit `i` set when `ATTRS[i]` is on.
    attrs: u16,
}

pub type Cell = (char, Style);

impl Style {
    fn set(&mut self, attr: u16, on: bool) {
        if let Some(i) = ATTRS.iter().position(|&a| a == attr) {
            if on {
                self.attrs |= 1 << i;
            } else {
                self.attrs &= !(1 << i);
            }
        }
    }

    fn has(&self, attr: u16) -> bool {
        ATTRS
            .iter()
            .position(|&a| a == attr)
            .is_some_and(|i| self.attrs & (1 << i) != 0)
    }

    /// Whether a space in this style still draws something, so trimming it would
    /// change what the row looks like.
    fn shows_on_space(&self) -> bool {
        self.bg.is_some() || [4, 7, 9, 53].iter().any(|&a| self.has(a))
    }

    /// Apply one SGR sequence's parameters (`1;38;5;208`, `4:3`).
    fn apply(&mut self, params: &str) {
        let parts: Vec<&str> = params.split(';').collect();
        let mut i = 0;
        while i < parts.len() {
            let p = parts[i];
            i += 1;
            // Colon sub-parameters (`4:3`, `38:2::r:g:b`) carry their own arguments.
            if p.contains(':') {
                let subs: Vec<&str> = p.split(':').collect();
                match subs[0] {
                    "4" => self.set(4, subs[1] != "0"),
                    "38" => self.fg = colon_color(&subs).map(|c| format!("38;{}", c)),
                    "48" => self.bg = colon_color(&subs).map(|c| format!("48;{}", c)),
                    _ => {}
                }
                continue;
            }
            // An empty parameter means 0, as in `\e[m`.
            let n: u16 = if p.is_empty() {
                0
            } else {
                match p.parse() {
                    Ok(n) => n,
                    Err(_) => continue,
                }
            };
            match n {
                0 => *self = Style::default(),
                // `5;n` or `2;r;g;b` follow as parameters of their own.
                38 | 48 | 58 => {
                    let take = match parts.get(i) {
                        Some(&"5") => 2,
                        Some(&"2") => 4,
                        _ => 0,
                    };
                    if take == 0 || i + take > parts.len() {
                        continue;
                    }
                    let spec = format!("{};{}", n, parts[i..i + take].join(";"));
                    i += take;
                    match n {
                        38 => self.fg = Some(spec),
                        48 => self.bg = Some(spec),
                        _ => {} // underline color: not drawn
                    }
                }
                30..=37 | 90..=97 => self.fg = Some(n.to_string()),
                39 => self.fg = None,
                40..=47 | 100..=107 => self.bg = Some(n.to_string()),
                49 => self.bg = None,
                21 => self.set(4, true), // double underline, drawn as underline
                22 => {
                    self.set(1, false);
                    self.set(2, false);
                }
                23..=29 => self.set(n - 20, false),
                55 => self.set(53, false),
                _ => self.set(n, true),
            }
        }
    }

    /// The complete escape for this style: a reset, the defaults standing in for
    /// the pane's default colors, then whatever this style sets on top.
    fn sgr(&self, default_fg: &str, default_bg: &str) -> String {
        let mut params: Vec<String> = ATTRS
            .iter()
            .filter(|&&a| self.has(a))
            .map(|a| a.to_string())
            .collect();
        params.extend(self.fg.clone());
        params.extend(self.bg.clone());
        let mut out = format!("\x1b[0m{}{}", default_fg, default_bg);
        if !params.is_empty() {
            out.push_str(&format!("\x1b[{}m", params.join(";")));
        }
        out
    }
}

/// `5;n` or `2;r;g;b` from a colon-form color (`38:5:n`, `38:2::r:g:b`, or
/// `38:2:r:g:b` without the color-space slot), or `None` when malformed.
fn colon_color(subs: &[&str]) -> Option<String> {
    match subs.get(1).copied() {
        Some("5") => subs.get(2).map(|n| format!("5;{}", n)),
        Some("2") if subs.len() >= 5 => Some(format!("2;{}", subs[subs.len() - 3..].join(";"))),
        _ => None,
    }
}

fn is_blank(cell: &Cell) -> bool {
    cell.0 == ' ' && !cell.1.shows_on_space()
}

/// Parse captured screen text into rows of styled cells, carrying SGR state
/// across lines as tmux emits it. Escape sequences other than SGR are dropped,
/// as are characters that occupy no cell. Each row's trailing blank cells are
/// trimmed — what `capture-pane` does without `-N`, except that spaces with a
/// background (a status bar running to the edge) are kept.
pub fn parse_screen(capture: &str) -> Vec<Vec<Cell>> {
    let mut style = Style::default();
    capture
        .split('\n')
        .map(|line| {
            let mut row: Vec<Cell> = Vec::new();
            let mut chars = line.chars();
            while let Some(c) = chars.next() {
                if c != '\x1b' {
                    if char_width(c) > 0 {
                        row.push((c, style.clone()));
                    }
                    continue;
                }
                match chars.next() {
                    // CSI: parameter bytes, then a final byte in `@`..=`~`.
                    Some('[') => {
                        let mut params = String::new();
                        for c in chars.by_ref() {
                            if ('@'..='~').contains(&c) {
                                if c == 'm' {
                                    style.apply(&params);
                                }
                                break;
                            }
                            params.push(c);
                        }
                    }
                    // OSC (e.g. an OSC 8 hyperlink): up to BEL or ST (`ESC \`).
                    Some(']') => {
                        while let Some(c) = chars.next() {
                            if c == '\x07' {
                                break;
                            }
                            if c == '\x1b' {
                                chars.next();
                                break;
                            }
                        }
                    }
                    // nF escape (`ESC ( B`): intermediate bytes, then a final one.
                    Some(c) if (' '..='/').contains(&c) => {
                        for c in chars.by_ref() {
                            if !(' '..='/').contains(&c) {
                                break;
                            }
                        }
                    }
                    _ => {} // any other escape: dropped with its one byte
                }
            }
            while row.last().is_some_and(is_blank) {
                row.pop();
            }
            row
        })
        .collect()
}

/// Draw `row` in at most `width` cells: a hard cut, as a viewport clips, that
/// never splits a wide glyph. `default_fg` / `default_bg` (escape sequences)
/// stand in for the pane's default colors. The result's `display_width` is the
/// cells drawn, so it passes through `text::truncate` untouched.
pub fn render_row(row: &[Cell], width: i64, default_fg: &str, default_bg: &str) -> String {
    let mut out = String::new();
    let mut used = 0;
    let mut current: Option<&Style> = None;
    for (c, style) in row {
        used += char_width(*c);
        if used > width {
            break;
        }
        if current != Some(style) {
            out.push_str(&style.sgr(default_fg, default_bg));
            current = Some(style);
        }
        out.push(*c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::display_width;

    fn text(row: &[Cell]) -> String {
        row.iter().map(|(c, _)| c).collect()
    }

    fn style_of(sgr: &str) -> Style {
        let mut s = Style::default();
        s.apply(sgr);
        s
    }

    #[test]
    fn style_carries_from_one_line_into_the_next() {
        let rows = parse_screen("\x1b[31mred\nstill red\x1b[39m plain");
        assert_eq!(rows[0][0].1.fg.as_deref(), Some("31"));
        assert_eq!(rows[1][0].1.fg.as_deref(), Some("31"));
        assert_eq!(rows[1].last().unwrap().1, Style::default());
    }

    #[test]
    fn reads_basic_extended_and_colon_colors() {
        assert_eq!(style_of("38;5;208").fg.as_deref(), Some("38;5;208"));
        assert_eq!(style_of("48;2;1;2;3").bg.as_deref(), Some("48;2;1;2;3"));
        assert_eq!(
            style_of("38:2::255:0:0").fg.as_deref(),
            Some("38;2;255;0;0")
        );
        assert_eq!(style_of("38:2:255:0:0").fg.as_deref(), Some("38;2;255;0;0"));
        assert_eq!(style_of("48:5:17").bg.as_deref(), Some("48;5;17"));
        // A palette index that happens to be 39 is a color, not "default fg".
        assert_eq!(style_of("38;5;39").fg.as_deref(), Some("38;5;39"));
        // The underline color is consumed whole rather than misread as attributes.
        assert_eq!(style_of("58;2;9;9;9;1"), style_of("1"));
    }

    #[test]
    fn attributes_switch_on_and_off() {
        let s = style_of("1;3;4:3;7");
        assert!(s.has(1) && s.has(3) && s.has(4) && s.has(7));
        let mut off = s.clone();
        off.apply("22;23;4:0;27");
        assert_eq!(off, Style::default());
        // Empty and zero parameters both reset everything.
        for reset in ["", "0"] {
            let mut r = style_of("1;31;44");
            r.apply(reset);
            assert_eq!(r, Style::default());
        }
    }

    #[test]
    fn drops_hyperlinks_other_escapes_and_zero_width_characters() {
        let rows = parse_screen("\x1b]8;;http://x\x1b\\link\x1b]8;;\x07 a\x07b\x1b[2Kc\x1b(Bd");
        assert_eq!(text(&rows[0]), "link abcd");
    }

    #[test]
    fn trims_plain_trailing_spaces_but_keeps_a_colored_bar() {
        let rows = parse_screen("prompt   \n\x1b[44mbar   \x1b[0m   ");
        assert_eq!(text(&rows[0]), "prompt");
        assert_eq!(text(&rows[1]), "bar   ");
    }

    #[test]
    fn render_cuts_hard_without_splitting_a_wide_glyph() {
        let rows = parse_screen("ab日本");
        // 5 cells: `ab` and `日` fit, `本` would straddle the edge.
        let out = render_row(&rows[0], 5, "", "");
        assert_eq!(crate::text::strip(&out), "ab日");
        assert_eq!(display_width(&out), 4);
    }

    #[test]
    fn render_draws_defaults_in_the_given_colors_and_restyles_only_on_change() {
        let rows = parse_screen("ab\x1b[1;31mcd\x1b[0me");
        let out = render_row(&rows[0], 10, "\x1b[39m", "\x1b[49m");
        assert_eq!(
            out,
            "\x1b[0m\x1b[39m\x1b[49mab\x1b[0m\x1b[39m\x1b[49m\x1b[1;31mcd\x1b[0m\x1b[39m\x1b[49me"
        );
        assert_eq!(display_width(&out), 5);
    }
}
