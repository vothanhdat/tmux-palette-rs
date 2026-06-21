//! User configuration loader — port of `src/userConfig.ts`.
//!
//! Drop-in JSON config lives in `~/.config/tmux-palette/` (one file per
//! concern), so customizations survive upstream updates without source edits.
//! Files are read fresh on demand; missing/invalid files fall back to defaults.

use std::collections::{HashMap, HashSet};
use std::fs;

use serde::Deserialize;

use crate::types::{Action, Item, PopupAction};

/// `~/.config/tmux-palette` (honoring `XDG_CONFIG_HOME`, and a port-only
/// `TMUX_PALETTE_CONFIG_DIR` override that takes precedence — handy for tests).
pub fn config_dir() -> String {
    if let Ok(d) = std::env::var("TMUX_PALETTE_CONFIG_DIR") {
        if !d.is_empty() {
            return d;
        }
    }
    match std::env::var("XDG_CONFIG_HOME") {
        Ok(x) => format!("{}/tmux-palette", x),
        Err(_) => {
            let home = std::env::var("HOME").unwrap_or_default();
            format!("{}/.config/tmux-palette", home)
        }
    }
}

fn read_file(name: &str) -> Option<String> {
    fs::read_to_string(format!("{}/{}", config_dir(), name)).ok()
}

fn load_json<T: for<'de> Deserialize<'de>>(name: &str) -> Option<T> {
    let raw = read_file(name)?;
    serde_json::from_str::<T>(&raw).ok()
}

pub fn user_shortcuts() -> HashMap<String, String> {
    load_json("shortcuts.json").unwrap_or_default()
}

pub fn user_aliases() -> HashMap<String, Vec<String>> {
    load_json("aliases.json").unwrap_or_default()
}

pub fn user_hidden() -> HashSet<String> {
    let list: Vec<String> = load_json("hidden.json").unwrap_or_default();
    list.into_iter().collect()
}

pub fn user_commands() -> Vec<Item> {
    let raw: Vec<JsonItem> = load_json("commands.json").unwrap_or_default();
    raw.into_iter().map(Item::from).collect()
}

/// Parse a JSON array of `Item` objects (used by the plugin-command escape
/// hatch). Returns `None` when the text isn't a valid array of items.
pub fn parse_json_items(s: &str) -> Option<Vec<Item>> {
    let raw: Vec<JsonItem> = serde_json::from_str(s).ok()?;
    Some(raw.into_iter().map(Item::from).collect())
}

// ---- JSON item / action shapes ------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonPopup {
    popup: String,
    width: Option<String>,
    height: Option<String>,
    pad_x: Option<i64>,
    pad_y: Option<i64>,
    border: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum JsonAction {
    Tmux { tmux: String },
    Shell { shell: String },
    Palette { palette: String },
    Popup(JsonPopup),
}

impl From<JsonAction> for Action {
    fn from(a: JsonAction) -> Action {
        match a {
            JsonAction::Tmux { tmux } => Action::Tmux(tmux),
            JsonAction::Shell { shell } => Action::Shell(shell),
            JsonAction::Palette { palette } => Action::Palette(palette),
            JsonAction::Popup(p) => Action::Popup(PopupAction {
                popup: p.popup,
                width: p.width,
                height: p.height,
                pad_x: p.pad_x,
                pad_y: p.pad_y,
                border: p.border,
            }),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonItem {
    icon: Option<String>,
    icon_color: Option<String>,
    title: String,
    description: Option<String>,
    shortcut: Option<String>,
    category: Option<String>,
    aliases: Option<Vec<String>>,
    action: JsonAction,
    selectable: Option<bool>,
}

impl From<JsonItem> for Item {
    fn from(j: JsonItem) -> Item {
        Item {
            icon: j.icon,
            icon_color: j.icon_color,
            title: j.title,
            description: j.description,
            shortcut: j.shortcut,
            category: j.category,
            aliases: j.aliases,
            action: j.action.into(),
            data: None,
            selectable: j.selectable,
            query_only: false,
        }
    }
}

// ---- sizing.json --------------------------------------------------------------

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct Sizing {
    pub width: Option<i64>,
    pub max_height: Option<i64>,
    pub pad_x: Option<i64>,
    /// Below this client width the popup goes fullscreen. 0 disables.
    pub mobile_width: Option<i64>,
    /// Main palette border: none|single|double|heavy|rounded|padded|simple.
    pub border: Option<String>,
    pub body_style: Option<String>,
    pub border_style: Option<String>,
    pub popup_border: Option<String>,
    pub popup_body_style: Option<String>,
    pub popup_border_style: Option<String>,
    pub popup_width: Option<String>,
    pub popup_height: Option<String>,
    pub popup_pad_x: Option<i64>,
    pub popup_pad_y: Option<i64>,
    /// ESC in nested palettes: "back" (pop one level) or "exit".
    pub esc: Option<String>,
}

pub fn user_sizing() -> Sizing {
    load_json("sizing.json").unwrap_or_default()
}

// ---- custom palettes ----------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCustomPalette {
    title: Option<String>,
    items: Option<Vec<JsonItem>>,
    from: Option<Vec<String>>,
    from_category: Option<String>,
    command: Option<String>,
    action: Option<JsonAction>,
    icon: Option<String>,
    icon_color: Option<String>,
    grouped: Option<bool>,
    empty_text: Option<String>,
}

/// A `~/.config/tmux-palette/palettes/<name>.json` definition (converted to
/// the internal `Item`/`Action` types).
pub struct CustomPalette {
    pub title: Option<String>,
    pub items: Vec<Item>,
    pub from: Vec<String>,
    pub from_category: Option<String>,
    pub command: Option<String>,
    pub action: Option<Action>,
    pub icon: Option<String>,
    pub icon_color: Option<String>,
    pub grouped: Option<bool>,
    pub empty_text: Option<String>,
}

pub fn user_palette(name: &str) -> Option<CustomPalette> {
    let raw: RawCustomPalette = load_json(&format!("palettes/{}.json", name))?;
    Some(CustomPalette {
        title: raw.title,
        items: raw
            .items
            .unwrap_or_default()
            .into_iter()
            .map(Item::from)
            .collect(),
        from: raw.from.unwrap_or_default(),
        from_category: raw.from_category,
        command: raw.command,
        action: raw.action.map(Action::from),
        icon: raw.icon,
        icon_color: raw.icon_color,
        grouped: raw.grouped,
        empty_text: raw.empty_text,
    })
}
