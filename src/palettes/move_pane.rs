//! `move-pane` palette — relocate the current pane — port of
//! `src/palettes/move-pane.ts`.

use std::rc::Rc;

use crate::tmux::{tmux, tmux_quote};
use crate::types::{Action, Item, ItemsSource, PaletteDef};

fn new_window_items(sessions: &[String], pane_id: &str) -> Vec<Item> {
    sessions
        .iter()
        .map(|session| Item {
            icon: Some("\u{f0770}".to_string()),
            title: "New window".to_string(),
            description: Some(format!("in {}", session)),
            action: Action::Tmux(format!(
                "break-pane -d -s {} -t {}",
                tmux_quote(pane_id),
                tmux_quote(&format!("{}:", session))
            )),
            ..Default::default()
        })
        .collect()
}

fn parse_window_line(line: &str) -> Option<(String, String, String)> {
    let f: Vec<&str> = line.split('\t').collect();
    let session = f.first().copied().unwrap_or("");
    let window_index = f.get(1).copied().unwrap_or("");
    if session.is_empty() || window_index.is_empty() {
        return None;
    }
    let window_name = f.get(2).copied().unwrap_or("");
    let window_name = if window_name.is_empty() {
        format!("window{}", window_index)
    } else {
        window_name.to_string()
    };
    Some((session.to_string(), window_index.to_string(), window_name))
}

fn join_window_items(win_lines: &[String], pane_id: &str, current_window: &str) -> Vec<Item> {
    let mut items = Vec::new();
    for line in win_lines {
        let Some((session, window_index, window_name)) = parse_window_line(line) else {
            continue;
        };
        let target = format!("{}:{}", session, window_index);
        if target == current_window {
            continue;
        }
        items.push(Item {
            icon: Some("\u{f05b2}".to_string()),
            title: window_name,
            description: Some(format!("{} \u{b7} {}", session, window_index)),
            action: Action::Tmux(format!(
                "join-pane -d -s {} -t {}",
                tmux_quote(pane_id),
                tmux_quote(&target)
            )),
            ..Default::default()
        });
    }
    items
}

fn build_items() -> Vec<Item> {
    let pane_id = tmux(&["display-message", "-p", "#{pane_id}"]);
    let current_window = tmux(&["display-message", "-p", "#{session_name}:#{window_index}"]);
    let sessions: Vec<String> = tmux(&["list-sessions", "-F", "#S"])
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(|s| s.to_string())
        .collect();
    let win_lines: Vec<String> = tmux(&[
        "list-windows",
        "-a",
        "-F",
        "#{session_name}\t#{window_index}\t#{window_name}",
    ])
    .split('\n')
    .filter(|l| !l.is_empty())
    .map(|s| s.to_string())
    .collect();

    let mut items = new_window_items(&sessions, &pane_id);
    items.extend(join_window_items(&win_lines, &pane_id, &current_window));
    items
}

pub fn move_pane() -> PaletteDef {
    PaletteDef {
        title: Some("Move Pane to...".to_string()),
        grouped: Some(false),
        empty_text: Some("No targets".to_string()),
        items: ItemsSource::Dynamic(Rc::new(build_items)),
        ..Default::default()
    }
}
