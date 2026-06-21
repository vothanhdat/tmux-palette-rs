//! Theme resolution + ANSI/tmux color translation — port of `src/theme.ts`.

use std::collections::HashMap;
use std::fs;

use serde::Deserialize;

use crate::themes_bundled::{bundled_theme, bundled_themes, DEFAULT_SLUG};
use crate::types::{Colors, Theme, ThemeRef};
use crate::user_config::{config_dir, user_sizing};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThemeJson {
    bg: String,
    panel: String,
    selected: String,
    fg: String,
    muted: String,
    accent: String,
    selected_fg: Option<String>,
    title_fg: Option<String>,
}

impl From<ThemeJson> for Theme {
    fn from(j: ThemeJson) -> Theme {
        Theme {
            bg: j.bg,
            panel: j.panel,
            selected: j.selected,
            fg: j.fg,
            muted: j.muted,
            accent: j.accent,
            selected_fg: j.selected_fg,
            title_fg: j.title_fg,
        }
    }
}

/// User themes from `~/.config/tmux-palette/themes/*.json` keyed by slug
/// (filename without extension). Files missing a required color are ignored.
fn user_themes() -> HashMap<String, Theme> {
    let mut out = HashMap::new();
    let dir = format!("{}/themes", config_dir());
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.ends_with(".json") {
                continue;
            }
            let slug = name[..name.len() - 5].to_string();
            if let Ok(raw) = fs::read_to_string(&path) {
                if let Ok(parsed) = serde_json::from_str::<ThemeJson>(&raw) {
                    out.insert(slug, parsed.into());
                }
            }
        }
    }
    out
}

pub struct ThemeListEntry {
    pub slug: String,
    pub name: String,
    pub theme: Theme,
    pub source: &'static str,
}

/// All themes available to the switcher: user themes override bundled ones by
/// slug, sorted by display name.
pub fn list_themes() -> Vec<ThemeListEntry> {
    let user = user_themes();
    let user_slugs: std::collections::HashSet<String> = user.keys().cloned().collect();

    let mut entries: Vec<ThemeListEntry> = user
        .into_iter()
        .map(|(slug, theme)| ThemeListEntry {
            name: slug.clone(),
            slug,
            theme,
            source: "user",
        })
        .collect();

    for b in bundled_themes() {
        if user_slugs.contains(b.slug) {
            continue;
        }
        entries.push(ThemeListEntry {
            slug: b.slug.to_string(),
            name: b.name.to_string(),
            theme: b.theme,
            source: "bundled",
        });
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

/// Resolve a declared theme (slug or literal) into a `Theme`. Unknown slugs
/// fall back to the default theme (the TS version throws; the port keeps the
/// popup alive and reports the typo on stderr).
fn resolve_theme(theme: &Option<ThemeRef>) -> Theme {
    match theme {
        None => bundled_theme(DEFAULT_SLUG).unwrap(),
        Some(ThemeRef::Full(t)) => t.clone(),
        Some(ThemeRef::Name(slug)) => resolve_slug(slug),
    }
}

fn resolve_slug(slug: &str) -> Theme {
    if let Some(t) = user_themes().get(slug) {
        return t.clone();
    }
    if let Some(t) = bundled_theme(slug) {
        return t;
    }
    eprintln!("tmux-palette: unknown theme '{}', using default", slug);
    bundled_theme(DEFAULT_SLUG).unwrap()
}

/// Reads `~/.config/tmux-palette/theme.json`: `{ "name": "slug" }`, a full
/// theme, or a partial override.
fn user_theme_file() -> Option<serde_json::Value> {
    let raw = fs::read_to_string(format!("{}/theme.json", config_dir())).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Combine a palette's declared theme with the user's `theme.json`.
pub fn resolve_active_theme(declared: &Option<ThemeRef>) -> Theme {
    let file = user_theme_file();
    if let Some(serde_json::Value::Object(map)) = &file {
        if let Some(serde_json::Value::String(name)) = map.get("name") {
            return resolve_slug(name);
        }
        let mut base = resolve_theme(declared);
        apply_override(&mut base, map);
        return base;
    }
    resolve_theme(declared)
}

fn apply_override(theme: &mut Theme, map: &serde_json::Map<String, serde_json::Value>) {
    let get = |k: &str| map.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
    if let Some(v) = get("bg") {
        theme.bg = v;
    }
    if let Some(v) = get("panel") {
        theme.panel = v;
    }
    if let Some(v) = get("selected") {
        theme.selected = v;
    }
    if let Some(v) = get("fg") {
        theme.fg = v;
    }
    if let Some(v) = get("muted") {
        theme.muted = v;
    }
    if let Some(v) = get("accent") {
        theme.accent = v;
    }
    if let Some(v) = get("selectedFg") {
        theme.selected_fg = Some(v);
    }
    if let Some(v) = get("titleFg") {
        theme.title_fg = Some(v);
    }
}

// ---- color translation --------------------------------------------------------

fn parse_hex(value: &str) -> Option<(u8, u8, u8)> {
    let s = value.strip_prefix('#').unwrap_or(value);
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r, g, b))
}

fn fg_hex(value: &str) -> String {
    match parse_hex(value) {
        Some((r, g, b)) => format!("\x1b[38;2;{};{};{}m", r, g, b),
        None => "\x1b[39m".to_string(),
    }
}

fn bg_hex(value: &str) -> String {
    match parse_hex(value) {
        Some((r, g, b)) => format!("\x1b[48;2;{};{};{}m", r, g, b),
        None => "\x1b[49m".to_string(),
    }
}

const TRANSPARENT: &str = "transparent";

fn ansi_base(name: &str) -> Option<usize> {
    Some(match name {
        "black" => 0,
        "red" => 1,
        "green" => 2,
        "yellow" => 3,
        "blue" => 4,
        "magenta" => 5,
        "cyan" => 6,
        "white" => 7,
        _ => return None,
    })
}

struct Named {
    base: String,
    idx: usize,
    bright: bool,
}

fn named_color(value: &str) -> Option<Named> {
    let bright = value.starts_with("bright-");
    let base = if bright { &value[7..] } else { value };
    ansi_base(base).map(|idx| Named {
        base: base.to_string(),
        idx,
        bright,
    })
}

fn bg_or_default(value: &str) -> String {
    if value == TRANSPARENT {
        return "\x1b[49m".to_string();
    }
    if let Some(n) = named_color(value) {
        return format!("\x1b[{}m", (if n.bright { 100 } else { 40 }) + n.idx);
    }
    bg_hex(value)
}

fn fg_or_default(value: &str) -> String {
    if value == TRANSPARENT {
        return "\x1b[39m".to_string();
    }
    if let Some(n) = named_color(value) {
        return format!("\x1b[{}m", (if n.bright { 90 } else { 30 }) + n.idx);
    }
    fg_hex(value)
}

/// Translate a theme color into a tmux style value.
pub fn tmux_color(value: &str) -> String {
    if value == TRANSPARENT {
        return "default".to_string();
    }
    if let Some(n) = named_color(value) {
        return if n.bright {
            format!("bright{}", n.base)
        } else {
            n.base
        };
    }
    value.to_string()
}

fn is_hex6(s: &str) -> bool {
    let s = s.strip_prefix('#').unwrap_or(s);
    s.len() == 6 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// OSC 12 sequence tinting the terminal cursor to the accent (hex accents only).
pub fn cursor_tint(theme: &Theme) -> String {
    if is_hex6(&theme.accent) {
        format!("\x1b]12;{}\x07", theme.accent)
    } else {
        String::new()
    }
}

pub fn make_colors(theme: &Theme) -> Colors {
    Colors {
        bg: bg_or_default(&theme.bg),
        panel: bg_or_default(&theme.panel),
        selected: format!(
            "{}{}",
            bg_or_default(&theme.selected),
            fg_or_default(&theme.fg)
        ),
        fg: fg_or_default(&theme.fg),
        muted: fg_or_default(&theme.muted),
        accent: fg_or_default(&theme.accent),
        selected_fg: theme
            .selected_fg
            .as_deref()
            .map(fg_or_default)
            .unwrap_or_default(),
        title_fg: theme
            .title_fg
            .as_deref()
            .map(fg_or_default)
            .unwrap_or_default(),
        reset: "\x1b[0m".to_string(),
        bold: "\x1b[1m".to_string(),
    }
}

/// tmux display-popup body style (`-s`).
pub fn tmux_body_style(theme: &Theme) -> String {
    format!("bg={}", tmux_color(&theme.panel))
}

/// Builds tmux display-popup style flags: `-B -s` when borderless, else the
/// `-b/-s/-S` triplet. `border_override` (a per-action border) wins over
/// `sizing.popupBorder`.
pub fn popup_flags(theme: &Theme, border_override: Option<&str>) -> String {
    let sizing = user_sizing();
    let popup_border = border_override
        .map(|s| s.to_string())
        .or(sizing.popup_border)
        .unwrap_or_else(|| "none".to_string());
    let body_style = sizing
        .popup_body_style
        .unwrap_or_else(|| tmux_body_style(theme));
    if popup_border == "none" {
        return format!("-B -s '{}'", body_style);
    }
    let border_style = sizing
        .popup_border_style
        .unwrap_or_else(|| format!("fg={},bg=default", tmux_color(&theme.accent)));
    format!(
        "-b {} -s '{}' -S '{}'",
        popup_border, body_style, border_style
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_theme() -> Theme {
        Theme {
            bg: "#1a1b26".into(),
            panel: "#34354b".into(),
            selected: "#53567a".into(),
            fg: "#c0caf5".into(),
            muted: "#99a0bf".into(),
            accent: "#7aa2f7".into(),
            selected_fg: None,
            title_fg: None,
        }
    }

    #[test]
    fn emits_24bit_ansi_for_hex_themes() {
        let c = make_colors(&hex_theme());
        assert_eq!(c.panel, "\x1b[48;2;52;53;75m");
        assert_eq!(c.fg, "\x1b[38;2;192;202;245m");
        assert_eq!(c.selected, "\x1b[48;2;83;86;122m\x1b[38;2;192;202;245m");
    }

    #[test]
    fn transparent_backgrounds_use_default_bg() {
        let mut th = hex_theme();
        th.panel = "transparent".into();
        th.selected = "transparent".into();
        let c = make_colors(&th);
        assert_eq!(c.panel, "\x1b[49m");
        assert_eq!(c.selected, "\x1b[49m\x1b[38;2;192;202;245m");
    }

    #[test]
    fn transparent_foregrounds_use_default_fg() {
        let mut th = hex_theme();
        th.fg = "transparent".into();
        th.muted = "transparent".into();
        let c = make_colors(&th);
        assert_eq!(c.fg, "\x1b[39m");
        assert_eq!(c.muted, "\x1b[39m");
    }

    #[test]
    fn selected_fg_empty_when_unset_and_code_when_set() {
        assert_eq!(make_colors(&hex_theme()).selected_fg, "");
        let mut th = hex_theme();
        th.selected_fg = Some("#fabd2f".into());
        assert_eq!(make_colors(&th).selected_fg, "\x1b[38;2;250;189;47m");
    }

    #[test]
    fn maps_palette_color_names() {
        let mut th = hex_theme();
        th.accent = "blue".into();
        th.selected_fg = Some("yellow".into());
        th.muted = "bright-black".into();
        th.panel = "red".into();
        let c = make_colors(&th);
        assert_eq!(c.accent, "\x1b[34m");
        assert_eq!(c.selected_fg, "\x1b[33m");
        assert_eq!(c.muted, "\x1b[90m");
        assert_eq!(c.panel, "\x1b[41m");
    }

    #[test]
    fn maps_bright_background_names_to_10x() {
        let mut th = hex_theme();
        th.panel = "bright-blue".into();
        assert_eq!(make_colors(&th).panel, "\x1b[104m");
    }

    #[test]
    fn tmux_color_translates() {
        assert_eq!(tmux_color("transparent"), "default");
        assert_eq!(tmux_color("blue"), "blue");
        assert_eq!(tmux_color("bright-black"), "brightblack");
        assert_eq!(tmux_color("#1a1b26"), "#1a1b26");
    }

    #[test]
    fn cursor_tint_hex_only() {
        let mut th = hex_theme();
        th.accent = "#7aa2f7".into();
        assert_eq!(cursor_tint(&th), "\x1b]12;#7aa2f7\x07");
        th.accent = "blue".into();
        assert_eq!(cursor_tint(&th), "");
        th.accent = "transparent".into();
        assert_eq!(cursor_tint(&th), "");
    }

    #[test]
    fn tmux_body_style_uses_panel() {
        assert_eq!(tmux_body_style(&hex_theme()), "bg=#34354b");
        let mut th = hex_theme();
        th.panel = "transparent".into();
        assert_eq!(tmux_body_style(&th), "bg=default");
    }
}
