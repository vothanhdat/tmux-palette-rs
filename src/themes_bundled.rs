//! Curated bundled themes — port of `src/themes-bundled.ts`.
//!
//! Custom themes can still be added via `~/.config/tmux-palette/themes/*.json`.

use crate::types::Theme;

pub struct BundledTheme {
    pub slug: &'static str,
    pub name: &'static str,
    pub theme: Theme,
}

#[allow(clippy::too_many_arguments)]
fn t(
    bg: &str,
    panel: &str,
    selected: &str,
    fg: &str,
    muted: &str,
    accent: &str,
    selected_fg: Option<&str>,
    title_fg: Option<&str>,
) -> Theme {
    Theme {
        bg: bg.to_string(),
        panel: panel.to_string(),
        selected: selected.to_string(),
        fg: fg.to_string(),
        muted: muted.to_string(),
        accent: accent.to_string(),
        selected_fg: selected_fg.map(|s| s.to_string()),
        title_fg: title_fg.map(|s| s.to_string()),
    }
}

pub fn bundled_themes() -> Vec<BundledTheme> {
    vec![
        BundledTheme {
            slug: "shades-of-purple",
            name: "Shades of Purple",
            theme: t(
                "#1e1d40", "#2d2b55", "#504d7a", "#ffffff", "#a599e9", "#fad000", None, None,
            ),
        },
        BundledTheme {
            slug: "dracula",
            name: "Dracula",
            theme: t(
                "#282a36", "#45495d", "#6a6f8f", "#f8f8f2", "#bdc3d8", "#d6acff", None, None,
            ),
        },
        BundledTheme {
            slug: "tokyo-night",
            name: "Tokyo Night",
            theme: t(
                "#1a1b26", "#34354b", "#53567a", "#c0caf5", "#99a0bf", "#7aa2f7", None, None,
            ),
        },
        BundledTheme {
            slug: "catppuccin-mocha",
            name: "Catppuccin Mocha",
            theme: t(
                "#1e1e2e", "#383857", "#5a5a8b", "#cdd6f4", "#a6a9b9", "#89b4fa", None, None,
            ),
        },
        BundledTheme {
            slug: "gruvbox-dark",
            name: "Gruvbox Dark",
            theme: t(
                "#282828", "#414141", "#646464", "#ebdbb2", "#b7ada4", "#8ec07c", None, None,
            ),
        },
        BundledTheme {
            slug: "rose-pine",
            name: "Rosé Pine",
            theme: t(
                "#191724", "#3c3857", "#645c8f", "#e0def4", "#b1aebf", "#9ccfd8", None, None,
            ),
        },
        BundledTheme {
            slug: "nord",
            name: "Nord",
            theme: t(
                "#2e3440", "#3f4758", "#5c677f", "#d8dee9", "#abb2c0", "#88c0d0", None, None,
            ),
        },
        BundledTheme {
            slug: "solarized-dark",
            name: "Solarized Dark",
            theme: t(
                "#002b36", "#00333f", "#00485b", "#839496", "#4a8897", "#268bd2", None, None,
            ),
        },
        BundledTheme {
            slug: "kanagawa-wave",
            name: "Kanagawa Wave",
            theme: t(
                "#1f1f28", "#3a3a4b", "#5c5c77", "#dcd7ba", "#b4aa6c", "#7e9cd8", None, None,
            ),
        },
        BundledTheme {
            slug: "github-dark",
            name: "GitHub Dark",
            theme: t(
                "#101216", "#1e2129", "#363c4a", "#8b949e", "#707a85", "#6ca4f8", None, None,
            ),
        },
        BundledTheme {
            slug: "one-dark",
            name: "One Dark",
            theme: t(
                "#21252b", "#2f353d", "#48505e", "#abb2bf", "#8691a3", "#61afef", None, None,
            ),
        },
        BundledTheme {
            slug: "ayu-dark",
            name: "Ayu Dark",
            theme: t(
                "#0b0e14", "#242e41", "#3f5072", "#bfbdb6", "#98958a", "#53bdfa", None, None,
            ),
        },
        // Fully terminal-native: transparent backgrounds, default-foreground
        // text, terminal-blue icons/title, and a terminal-yellow active row.
        BundledTheme {
            slug: "terminal",
            name: "Terminal",
            theme: t(
                "transparent",
                "transparent",
                "transparent",
                "transparent",
                "transparent",
                "blue",
                Some("yellow"),
                Some("blue"),
            ),
        },
    ]
}

pub const DEFAULT_SLUG: &str = "shades-of-purple";

/// Look up a bundled theme by slug.
pub fn bundled_theme(slug: &str) -> Option<Theme> {
    bundled_themes()
        .into_iter()
        .find(|t| t.slug == slug)
        .map(|t| t.theme)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_theme_resolves_native_colors() {
        let terminal = bundled_theme("terminal").expect("terminal theme exists");
        assert_eq!(terminal.bg, "transparent");
        assert_eq!(terminal.panel, "transparent");
        assert_eq!(terminal.selected, "transparent");
        assert_eq!(terminal.fg, "transparent");
        assert_eq!(terminal.muted, "transparent");
        assert_eq!(terminal.accent, "blue");
        assert_eq!(terminal.title_fg.as_deref(), Some("blue"));
    }
}
