//! Palette resolution, plugin commands, and popup sizing — port of the
//! non-interactive parts of `src/cli.ts`.

use std::io::Read;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::palette::PaletteLoader;
use crate::palettes::commands::commands;
use crate::palettes::find_pane::{find_pane, inline_pane_items};
use crate::palettes::move_pane::move_pane;
use crate::palettes::themes::themes;
use crate::theme::{resolve_active_theme, tmux_body_style, tmux_color};
use crate::types::{Action, Item, ItemsSource, PaletteDef};
use crate::user_config::{
    parse_json_items, user_commands, user_hidden, user_palette, user_sizing, CustomPalette,
};

fn substitute_template(action: &Action, value: &str) -> Action {
    match action {
        Action::Shell(s) => Action::Shell(s.replace("{}", value)),
        Action::Tmux(s) => Action::Tmux(s.replace("{}", value)),
        Action::Popup(p) => {
            let mut p = p.clone();
            p.popup = p.popup.replace("{}", value);
            Action::Popup(p)
        }
        other => other.clone(),
    }
}

fn opt(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.is_empty())
}

fn lines_to_items(
    out: &str,
    template: &Action,
    default_icon: &Option<String>,
    default_icon_color: &Option<String>,
) -> Vec<Item> {
    out.split('\n')
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            // <icon>\t<color>\t<title> | <icon>\t<title> | <title>
            let parts: Vec<&str> = line.split('\t').collect();
            let (icon, icon_color, title) = match parts.len() {
                1 => (
                    default_icon.clone(),
                    default_icon_color.clone(),
                    parts[0].to_string(),
                ),
                2 => (
                    Some(parts[0].to_string()),
                    default_icon_color.clone(),
                    parts[1].to_string(),
                ),
                _ => (
                    Some(parts[0].to_string()),
                    Some(parts[1].to_string()),
                    parts[2..].join("\t"),
                ),
            };
            Item {
                icon: opt(icon),
                icon_color: opt(icon_color),
                action: substitute_template(template, &title),
                title,
                ..Default::default()
            }
        })
        .collect()
}

fn error_item(title: &str, description: &str) -> Item {
    Item {
        icon: Some("".to_string()),
        title: title.to_string(),
        description: Some(description.to_string()),
        action: Action::Shell(":".to_string()),
        ..Default::default()
    }
}

/// Run `sh -c <cmd>` capturing stdout/stderr, killing it after `secs`.
fn run_capture_timeout(cmd: &str, secs: u64) -> Option<(i32, String, String)> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut so = child.stdout.take().unwrap();
    let mut se = child.stderr.take().unwrap();
    let h1 = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = so.read_to_string(&mut s);
        s
    });
    let h2 = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = se.read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = h1.join().unwrap_or_default();
                let err = h2.join().unwrap_or_default();
                return Some((status.code().unwrap_or(-1), out, err));
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let out = h1.join().unwrap_or_default();
                    let err = h2.join().unwrap_or_default();
                    return Some((-1, out, err));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    }
}

fn run_plugin_command(
    command: &str,
    template: &Option<Action>,
    icon: &Option<String>,
    icon_color: &Option<String>,
) -> Vec<Item> {
    let Some((status, out, err)) = run_capture_timeout(command, 10) else {
        return vec![error_item("Plugin command failed", "spawn error")];
    };
    if status != 0 {
        let msg = err
            .trim()
            .lines()
            .next()
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .unwrap_or_else(|| format!("exit {}", status));
        return vec![error_item("Plugin command failed", &msg)];
    }
    let out = out.trim();
    // JSON-array-of-objects mode (full Item control).
    if let Some(items) = parse_json_items(out) {
        return items;
    }
    // Plain-text mode: one item per line, action is the palette's template.
    match template {
        Some(t) => lines_to_items(out, t, icon, icon_color),
        None => vec![error_item(
            "Plain-text plugin output but no 'action' template set",
            "Add an 'action' field to the palette JSON (use {} for the line text)",
        )],
    }
}

fn build_custom_palette(name: &str) -> Option<PaletteDef> {
    let custom: CustomPalette = user_palette(name)?;
    let base_commands = commands().resolve_items();
    let mut all_main = base_commands;
    all_main.extend(user_commands());

    let referenced: Vec<Item> = custom
        .from
        .iter()
        .filter_map(|title| all_main.iter().find(|i| &i.title == title).cloned())
        .collect();
    let by_category: Vec<Item> = match &custom.from_category {
        Some(fc) => all_main
            .iter()
            .filter(|i| i.category.as_deref() == Some(fc.as_str()))
            .cloned()
            .collect(),
        None => Vec::new(),
    };
    let plugin_items: Vec<Item> = match &custom.command {
        Some(cmd) => run_plugin_command(cmd, &custom.action, &custom.icon, &custom.icon_color),
        None => Vec::new(),
    };

    let mut items = referenced;
    items.extend(by_category);
    items.extend(plugin_items);
    items.extend(custom.items);

    Some(PaletteDef {
        title: Some(custom.title.unwrap_or_else(|| name.to_string())),
        grouped: Some(custom.grouped.unwrap_or(false)),
        empty_text: custom.empty_text,
        items: ItemsSource::Static(items),
        ..Default::default()
    })
}

fn apply_commands_overrides(def: PaletteDef) -> PaletteDef {
    let extras = user_commands();
    let hidden = user_hidden();
    let base_items = def.resolve_items();
    let mut merged = base_items;
    merged.extend(extras);
    merged.retain(|i| !hidden.contains(&i.title));
    PaletteDef {
        items: ItemsSource::Static(merged),
        ..def
    }
}

/// Resolve a palette by name: built-in registry → custom palette JSON.
pub fn load_palette(name: &str) -> Option<PaletteDef> {
    let def = match name {
        "commands" => Some(commands()),
        "find-pane" => Some(find_pane()),
        "move-pane" => Some(move_pane()),
        "themes" => Some(themes()),
        _ => build_custom_palette(name),
    }?;
    if name == "commands" {
        Some(apply_commands_overrides(def))
    } else {
        Some(def)
    }
}

/// Loader passed into the runner for in-process sub-palette navigation.
pub fn make_loader() -> PaletteLoader {
    Rc::new(load_palette)
}

/// Append live panes as query-only inline items, so typing in the main palette
/// searches panes directly instead of requiring a hop through Find Pane. Kept
/// out of `load_palette`/`measure` so popup sizing and custom palettes are
/// unaffected and only the interactive instance pays the `tmux list-panes` cost.
pub fn with_inline_panes(def: PaletteDef) -> PaletteDef {
    let mut items = def.resolve_items();
    items.extend(inline_pane_items());
    PaletteDef {
        items: ItemsSource::Static(items),
        ..def
    }
}

/// Apply `--category=<name>` to a resolved palette: filter items to that
/// category, retitle, and ungroup.
pub fn apply_category(def: PaletteDef, category: &str) -> PaletteDef {
    let base_items = def.resolve_items();
    let filtered: Vec<Item> = base_items
        .into_iter()
        .filter(|i| i.category.as_deref() == Some(category))
        .collect();
    PaletteDef {
        items: ItemsSource::Static(filtered),
        title: Some(category.to_string()),
        grouped: Some(false),
        ..def
    }
}

pub struct Measurement {
    pub rows: i64,
    pub width: i64,
    pub pad_x: i64,
    pub border: String,
    pub body_style: String,
    pub border_style: String,
}

const DEFAULT_WIDTH: i64 = 90;
const DEFAULT_MAX_HEIGHT: i64 = 28;
const DEFAULT_PAD_X: i64 = 3;
const DEFAULT_MOBILE_WIDTH: i64 = 80;

/// Compute the popup geometry the palette wants (defaults + sizing.json),
/// triggering fullscreen mobile mode when the client is narrow.
pub fn measure(def: &PaletteDef, cw: i64, ch: i64) -> Measurement {
    let items = def.resolve_items();
    let grouped = def.grouped != Some(false);
    let cats = if grouped {
        let mut seen = std::collections::HashSet::new();
        items
            .iter()
            .filter_map(|i| i.category.as_deref())
            .filter(|c| !c.is_empty())
            .filter(|c| seen.insert(c.to_string()))
            .count() as i64
    } else {
        0
    };

    let sizing = user_sizing();
    let max_height = sizing.max_height.unwrap_or(DEFAULT_MAX_HEIGHT);
    let width = sizing.width.unwrap_or(DEFAULT_WIDTH);
    let pad_x = sizing.pad_x.unwrap_or(DEFAULT_PAD_X);
    let mobile_width = sizing.mobile_width.unwrap_or(DEFAULT_MOBILE_WIDTH);
    let border = sizing.border.unwrap_or_else(|| "none".to_string());

    let theme = resolve_active_theme(&def.theme);
    let body_style = sizing.body_style.unwrap_or_else(|| tmux_body_style(&theme));
    let border_style = sizing
        .border_style
        .unwrap_or_else(|| format!("fg={},bg=default", tmux_color(&theme.accent)));

    // chrome: top pad + header + search + spacer + footer spacer + footer + bottom pad = 7
    let desired = items.len() as i64 + cats + 7;
    let mut rows = desired.min(max_height);
    let mut final_width = width;
    let mut final_pad_x = pad_x;

    if mobile_width > 0 && cw > 0 && cw < mobile_width {
        rows = rows.max(ch);
        final_width = cw;
        final_pad_x = 1;
    }

    Measurement {
        rows,
        width: final_width,
        pad_x: final_pad_x,
        border,
        body_style,
        border_style,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_template_placeholders() {
        if let Action::Shell(s) = substitute_template(&Action::Shell("echo {}".into()), "hi") {
            assert_eq!(s, "echo hi");
        } else {
            panic!();
        }
        if let Action::Tmux(s) =
            substitute_template(&Action::Tmux("send-keys '{}' Enter".into()), "ls")
        {
            assert_eq!(s, "send-keys 'ls' Enter");
        } else {
            panic!();
        }
    }

    #[test]
    fn lines_to_items_parses_columns() {
        let tmpl = Action::Tmux("checkout {}".into());
        let items = lines_to_items("main\nfeat\ti2\n\tonly-title", &tmpl, &None, &None);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].title, "main");
        assert_eq!(items[0].icon, None);
        assert_eq!(items[1].icon.as_deref(), Some("feat"));
        assert_eq!(items[1].title, "i2");
        // empty leading icon field -> None
        assert_eq!(items[2].icon, None);
        assert_eq!(items[2].title, "only-title");
    }

    #[test]
    fn loads_builtin_palettes() {
        assert!(load_palette("commands").is_some());
        assert!(load_palette("find-pane").is_some());
        assert!(load_palette("move-pane").is_some());
        assert!(load_palette("themes").is_some());
        assert!(load_palette("nonexistent-xyz").is_none());
    }

    #[test]
    fn measure_includes_chrome_and_categories() {
        let def = commands();
        let m = measure(&def, 200, 50);
        // commands has 31 items across 6 categories; rows capped at maxHeight 28.
        assert_eq!(m.rows, 28);
        assert_eq!(m.width, 90);
        assert_eq!(m.pad_x, 3);
        assert_eq!(m.border, "none");
    }

    #[test]
    fn measure_triggers_mobile_fullscreen() {
        let def = commands();
        let m = measure(&def, 50, 40);
        assert_eq!(m.width, 50);
        assert_eq!(m.pad_x, 1);
        assert!(m.rows >= 40);
    }
}
