//! `find-pane` palette — tree of sessions/windows/panes — port of
//! `src/palettes/find-pane.ts`.

use std::any::Any;
use std::rc::Rc;

use crate::fuzzy::multi_fuzzy_score;
use crate::tmux::{tmux, tmux_quote};
use crate::types::{Action, Colors, Item, ItemsSource, PaletteDef, RenderItemCtx};

const SPINNER: &[char] = &[
    '*', '✳', '⠂', '⠐', '⠁', '⠉', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏',
];

fn detect_agent(command: &str, title: &str) -> String {
    const DIRECT: &[&str] = &[
        "claude",
        "codex",
        "aider",
        "cursor-agent",
        "opencode",
        "gemini",
        "ollama",
    ];
    if DIRECT.contains(&command) {
        return command.to_string();
    }
    if title.starts_with("OC | ") || title.starts_with("OC|") {
        return "opencode".to_string();
    }
    // /^\s*[<spinner>]\s/
    let chars: Vec<char> = title.chars().collect();
    let mut i = 0;
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    if i < chars.len()
        && SPINNER.contains(&chars[i])
        && i + 1 < chars.len()
        && chars[i + 1].is_whitespace()
    {
        return "claude".to_string();
    }
    String::new()
}

#[derive(Clone)]
pub struct Pane {
    pub session: String,
    pub window_index: String,
    pub pane_index: String,
    pub window_name: String,
    pub pane_title: String,
    pub command: String,
    pub path: String,
    pub agent: String,
    pub pane_active: bool,
    pub window_active: bool,
    pub is_current: bool,
    pub target: String,
}

#[derive(Clone)]
pub enum ItemData {
    Session {
        session: String,
        count: usize,
        path: String,
        is_current: bool,
    },
    Window {
        session: String,
        window_index: String,
        window_name: String,
        tree_prefix: String,
    },
    Pane {
        pane: Pane,
        tree_prefix: String,
    },
}

fn data(d: ItemData) -> Option<Rc<dyn Any>> {
    Some(Rc::new(d))
}

const PANE_FORMAT: &str = "#{session_name}\t#{window_index}\t#{pane_index}\t#{window_name}\t#{pane_title}\t#{pane_current_command}\t#{pane_current_path}\t#{pane_active}\t#{window_active}";

fn parse_pane_line(line: &str, current_pane: &str) -> Option<Pane> {
    let f: Vec<&str> = line.split('\t').collect();
    let session = f.first().copied().unwrap_or("");
    let window_index = f.get(1).copied().unwrap_or("");
    let pane_index = f.get(2).copied().unwrap_or("");
    if session.is_empty() || window_index.is_empty() || pane_index.is_empty() {
        return None;
    }
    let window_name = f.get(3).copied().unwrap_or("");
    let pane_title = f.get(4).copied().unwrap_or("");
    let command = f.get(5).copied().unwrap_or("");
    let path = f.get(6).copied().unwrap_or("");
    let pane_active = f.get(7).copied().unwrap_or("");
    let window_active = f.get(8).copied().unwrap_or("");

    let target = format!("{}:{}.{}", session, window_index, pane_index);
    let title = if pane_title.is_empty() {
        format!("pane{}", pane_index)
    } else {
        pane_title.to_string()
    };
    Some(Pane {
        session: session.to_string(),
        window_index: window_index.to_string(),
        pane_index: pane_index.to_string(),
        window_name: if window_name.is_empty() {
            format!("window{}", window_index)
        } else {
            window_name.to_string()
        },
        agent: detect_agent(command, &title),
        pane_title: title,
        command: command.to_string(),
        path: path.to_string(),
        pane_active: pane_active == "1",
        window_active: window_active == "1",
        is_current: target == current_pane,
        target,
    })
}

struct Fetched {
    panes: Vec<Pane>,
    current_session: String,
}

fn fetch_panes() -> Fetched {
    let current_pane = tmux(&[
        "display-message",
        "-p",
        "#{session_name}:#{window_index}.#{pane_index}",
    ]);
    let current_session = current_pane.split(':').next().unwrap_or("").to_string();
    let listing = tmux(&["list-panes", "-a", "-F", PANE_FORMAT]);
    let panes = listing
        .split('\n')
        .filter(|l| !l.is_empty())
        .filter_map(|l| parse_pane_line(l, &current_pane))
        .collect();
    Fetched {
        panes,
        current_session,
    }
}

struct WindowGroup {
    window_name: String,
    panes: Vec<Pane>,
}

/// Group panes by session, then window, preserving first-seen order.
fn group_panes(panes: Vec<Pane>) -> Vec<(String, Vec<(String, WindowGroup)>)> {
    let mut sessions: Vec<(String, Vec<(String, WindowGroup)>)> = Vec::new();
    for p in panes {
        let si = match sessions.iter().position(|(s, _)| *s == p.session) {
            Some(i) => i,
            None => {
                sessions.push((p.session.clone(), Vec::new()));
                sessions.len() - 1
            }
        };
        let windows = &mut sessions[si].1;
        let wi = match windows.iter().position(|(w, _)| *w == p.window_index) {
            Some(i) => i,
            None => {
                windows.push((
                    p.window_index.clone(),
                    WindowGroup {
                        window_name: p.window_name.clone(),
                        panes: Vec::new(),
                    },
                ));
                windows.len() - 1
            }
        };
        windows[wi].1.panes.push(p);
    }
    sessions
}

fn session_item(session: &str, all_in_session: &[Pane], current_session: &str) -> Item {
    let focused = all_in_session
        .iter()
        .find(|p| p.pane_active && p.window_active)
        .or_else(|| all_in_session.first());
    Item {
        title: session.to_string(),
        action: Action::Tmux(format!("switch-client -t {}", tmux_quote(session))),
        selectable: Some(false),
        data: data(ItemData::Session {
            session: session.to_string(),
            count: all_in_session.len(),
            path: focused.map(|p| p.path.clone()).unwrap_or_default(),
            is_current: session == current_session,
        }),
        ..Default::default()
    }
}

fn pane_select_action(p: &Pane) -> Action {
    let window_target = format!("{}:{}", p.session, p.window_index);
    Action::Tmux(format!(
        "select-pane -t {} \\; select-window -t {} \\; switch-client -t {}",
        tmux_quote(&p.target),
        tmux_quote(&window_target),
        tmux_quote(&p.session)
    ))
}

fn pane_item(p: &Pane, tree_prefix: String) -> Item {
    Item {
        title: p.pane_title.clone(),
        action: pane_select_action(p),
        data: data(ItemData::Pane {
            pane: p.clone(),
            tree_prefix,
        }),
        ..Default::default()
    }
}

fn window_item(session: &str, window_index: &str, w: &WindowGroup, tree_prefix: String) -> Item {
    Item {
        title: w.window_name.clone(),
        action: Action::Tmux(format!(
            "select-window -t {} \\; switch-client -t {}",
            tmux_quote(&format!("{}:{}", session, window_index)),
            tmux_quote(session)
        )),
        selectable: Some(false),
        data: data(ItemData::Window {
            session: session.to_string(),
            window_index: window_index.to_string(),
            window_name: w.window_name.clone(),
            tree_prefix,
        }),
        ..Default::default()
    }
}

fn window_subtree(
    session: &str,
    window_index: &str,
    w: &WindowGroup,
    is_last_win: bool,
) -> Vec<Item> {
    let win_prefix = format!("  {} ", if is_last_win { "└─" } else { "├─" });
    if w.panes.len() == 1 {
        return vec![pane_item(&w.panes[0], win_prefix)];
    }
    let mut items = vec![window_item(session, window_index, w, win_prefix)];
    let pane_prefix_base = if is_last_win { "      " } else { "  │   " };
    for (pi, p) in w.panes.iter().enumerate() {
        let is_last_pane = pi == w.panes.len() - 1;
        let tail = if is_last_pane { "└─ " } else { "├─ " };
        items.push(pane_item(p, format!("{}{}", pane_prefix_base, tail)));
    }
    items
}

fn build_items() -> Vec<Item> {
    let Fetched {
        panes,
        current_session,
    } = fetch_panes();
    let grouped = group_panes(panes);

    let mut items = Vec::new();
    for (session, windows) in &grouped {
        let all_in_session: Vec<Pane> = windows
            .iter()
            .flat_map(|(_, w)| w.panes.iter().cloned())
            .collect();
        items.push(session_item(session, &all_in_session, &current_session));
        let win_count = windows.len();
        for (wi, (window_index, w)) in windows.iter().enumerate() {
            items.extend(window_subtree(
                session,
                window_index,
                w,
                wi == win_count - 1,
            ));
        }
    }
    items
}

/// Marker glyph + optional color for a pane row rendered with the *default*
/// item renderer (used by the inline panes in the main palette).
fn pane_inline_icon(p: &Pane) -> (&'static str, Option<String>) {
    if p.is_current {
        ("▶", None)
    } else if p.pane_active {
        ("●", Some("#a6e3a1".to_string()))
    } else {
        ("○", None)
    }
}

/// Searchable, human-readable context for a pane: location plus the bits the
/// dedicated Find Pane filter also matches (window, command, agent, path).
fn pane_inline_description(p: &Pane) -> String {
    // Order matters (location first); skip empties and tokens already shown
    // (e.g. `pane_title == command`, or `agent == command` for AI tools).
    let mut ctx: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        if !s.is_empty() && !ctx.iter().any(|e| e == s) && s != p.pane_title {
            ctx.push(s.to_string());
        }
    };
    push(&p.target);
    push(&p.window_name);
    push(&p.command);
    push(&p.agent);
    push(&shorten_path(&p.path));
    ctx.join("  ")
}

fn panes_to_inline_items(panes: &[Pane]) -> Vec<Item> {
    panes
        .iter()
        .map(|p| {
            let (icon, icon_color) = pane_inline_icon(p);
            Item {
                icon: Some(icon.to_string()),
                icon_color,
                title: p.pane_title.clone(),
                description: Some(pane_inline_description(p)),
                action: pane_select_action(p),
                query_only: true,
                ..Default::default()
            }
        })
        .collect()
}

/// Live panes as flat, query-only items so the main palette can search panes
/// without first entering the Find Pane sub-palette. Hidden until the user
/// types (see `query_only`), then ranked by the default fuzzy filter.
pub fn inline_pane_items() -> Vec<Item> {
    let Fetched { panes, .. } = fetch_panes();
    panes_to_inline_items(&panes)
}

fn shorten_path(path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() && path.starts_with(&home) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_string()
    }
}

fn clen(s: &str) -> i64 {
    s.chars().count() as i64
}

fn render_session(
    session: &str,
    count: usize,
    path: &str,
    is_current: bool,
    colors: &Colors,
    row_bg: &str,
) -> String {
    let marker = if is_current {
        format!("{}▶ {}{}", colors.accent, colors.reset, row_bg)
    } else {
        "  ".to_string()
    };
    let name = format!(
        "{}{}{}{}{}",
        colors.accent, colors.bold, session, colors.reset, row_bg
    );
    let count = format!("{} ({}){}{}", colors.muted, count, colors.reset, row_bg);
    let path = if path.is_empty() {
        String::new()
    } else {
        format!(
            "  {}{}{}{}",
            colors.muted,
            shorten_path(path),
            colors.reset,
            row_bg
        )
    };
    format!("{}{}{}{}", marker, name, count, path)
}

fn render_window(
    window_name: &str,
    tree_prefix: &str,
    colors: &Colors,
    row_bg: &str,
    active: bool,
) -> String {
    let title_style = if active {
        format!("{}{}", colors.bold, colors.fg)
    } else {
        colors.fg.clone()
    };
    format!(
        "{}{}{}{}{}{}{}{}",
        colors.muted,
        tree_prefix,
        colors.reset,
        row_bg,
        title_style,
        window_name,
        colors.reset,
        row_bg
    )
}

fn pane_marker(p: &Pane, colors: &Colors) -> (String, char) {
    if p.is_current {
        (colors.accent.clone(), '▶')
    } else if p.pane_active {
        ("\x1b[38;2;166;227;161m".to_string(), '●')
    } else {
        (colors.muted.clone(), '○')
    }
}

fn render_pane(
    p: &Pane,
    tree_prefix: &str,
    colors: &Colors,
    row_bg: &str,
    active: bool,
    width: i64,
) -> String {
    let (marker_color, marker_char) = pane_marker(p, colors);
    let title_style = if active {
        format!("{}{}", colors.bold, colors.fg)
    } else if p.is_current {
        colors.fg.clone()
    } else {
        colors.muted.clone()
    };

    let mut left = format!(
        "{}{}{}{}{}{}{}{} {}{}{}{}",
        colors.muted,
        tree_prefix,
        colors.reset,
        row_bg,
        marker_color,
        marker_char,
        colors.reset,
        row_bg,
        title_style,
        p.pane_title,
        colors.reset,
        row_bg
    );
    let mut left_plain_w = clen(tree_prefix) + 1 + 1 + clen(&p.pane_title);

    if !p.agent.is_empty() {
        left += &format!("  {}{}{}{}", colors.muted, p.agent, colors.reset, row_bg);
        left_plain_w += 2 + clen(&p.agent);
    }

    let right_text = format!("{}.{}", p.window_index, p.pane_index);
    let right = format!("{}{}{}{}", colors.muted, right_text, colors.reset, row_bg);
    let gap = (width - left_plain_w - clen(&right_text)).max(1);
    format!("{}{}{}", left, " ".repeat(gap as usize), right)
}

fn render_item_impl(item: &Item, ctx: &RenderItemCtx) -> String {
    let colors = ctx.colors;
    let active = ctx.active;
    let row_bg = if active {
        &colors.selected
    } else {
        &colors.panel
    };
    let Some(data) = item
        .data
        .as_ref()
        .and_then(|d| d.downcast_ref::<ItemData>())
    else {
        return String::new();
    };
    match data {
        ItemData::Session {
            session,
            count,
            path,
            is_current,
        } => render_session(session, *count, path, *is_current, colors, row_bg),
        ItemData::Window {
            window_name,
            tree_prefix,
            ..
        } => render_window(window_name, tree_prefix, colors, row_bg, active),
        ItemData::Pane { pane, tree_prefix } => {
            render_pane(pane, tree_prefix, colors, row_bg, active, ctx.width)
        }
    }
}

fn filter_tree(items: &[Item], query: &str) -> Vec<Item> {
    let parts: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    if parts.is_empty() {
        return items.to_vec();
    }

    use std::collections::HashSet;
    let mut ok_sessions: HashSet<String> = HashSet::new();
    let mut ok_windows: HashSet<String> = HashSet::new();
    let mut ok_panes: HashSet<String> = HashSet::new();

    for item in items {
        if let Some(ItemData::Pane { pane: p, .. }) = item
            .data
            .as_ref()
            .and_then(|d| d.downcast_ref::<ItemData>())
        {
            let haystack = [
                &p.session,
                &p.window_name,
                &p.pane_title,
                &p.command,
                &p.path,
                &p.target,
                &p.agent,
            ]
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" ");
            if multi_fuzzy_score(&haystack, &parts) > 0 {
                ok_panes.insert(p.target.clone());
                ok_sessions.insert(p.session.clone());
                ok_windows.insert(format!("{}:{}", p.session, p.window_index));
            }
        }
    }

    items
        .iter()
        .filter(|item| {
            match item
                .data
                .as_ref()
                .and_then(|d| d.downcast_ref::<ItemData>())
            {
                Some(ItemData::Session { session, .. }) => ok_sessions.contains(session),
                Some(ItemData::Window {
                    session,
                    window_index,
                    ..
                }) => ok_windows.contains(&format!("{}:{}", session, window_index)),
                Some(ItemData::Pane { pane, .. }) => ok_panes.contains(&pane.target),
                None => false,
            }
        })
        .cloned()
        .collect()
}

fn initial_selected(items: &[Item]) -> i64 {
    items
        .iter()
        .position(|i| {
            matches!(
                i.data.as_ref().and_then(|d| d.downcast_ref::<ItemData>()),
                Some(ItemData::Pane { pane, .. }) if pane.is_current
            )
        })
        .map(|i| i as i64)
        .unwrap_or(-1)
}

pub fn find_pane() -> PaletteDef {
    PaletteDef {
        title: Some("Find Pane".to_string()),
        grouped: Some(false),
        empty_text: Some("No panes".to_string()),
        items: ItemsSource::Dynamic(Rc::new(build_items)),
        render_item: Some(Rc::new(render_item_impl)),
        filter: Some(Rc::new(filter_tree)),
        initial_selected: Some(Rc::new(initial_selected)),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_agents() {
        assert_eq!(detect_agent("claude", "x"), "claude");
        assert_eq!(detect_agent("vim", "OC | foo"), "opencode");
        assert_eq!(detect_agent("node", "✳ thinking"), "claude");
        assert_eq!(detect_agent("bash", "just a shell"), "");
    }

    #[test]
    fn parses_pane_line() {
        let line = "main\t0\t1\teditor\tnvim\tnvim\t/home/u/p\t1\t1";
        let p = parse_pane_line(line, "main:0.1").unwrap();
        assert_eq!(p.session, "main");
        assert_eq!(p.target, "main:0.1");
        assert!(p.is_current);
        assert!(p.pane_active);
        assert_eq!(p.pane_title, "nvim");
    }

    #[test]
    fn inline_items_are_query_only_and_searchable() {
        let panes = vec![
            parse_pane_line("main\t1\t0\teditor\tnvim\tnvim\t/home/u/proj\t1\t1", "main:1.0")
                .unwrap(),
            parse_pane_line("work\t0\t2\tshell\tbash\tbash\t/tmp\t0\t0", "main:1.0").unwrap(),
        ];
        let items = panes_to_inline_items(&panes);
        assert_eq!(items.len(), 2);

        // Current pane: marker ▶, title from pane_title, query-only.
        assert!(items[0].query_only);
        assert_eq!(items[0].title, "nvim");
        assert_eq!(items[0].icon.as_deref(), Some("▶"));
        // Location + window are searchable via the description.
        let desc0 = items[0].description.as_deref().unwrap();
        assert!(desc0.contains("main:1.0"));
        assert!(desc0.contains("editor"));
        // Selecting performs the pane switch.
        assert!(matches!(&items[0].action, Action::Tmux(c) if c.contains("select-pane -t 'main:1.0'")));

        // Inactive pane: hollow marker, no color override.
        assert_eq!(items[1].icon.as_deref(), Some("○"));
        assert_eq!(items[1].icon_color, None);
    }

    #[test]
    fn pane_select_uses_escaped_separators() {
        let p = parse_pane_line("s\t2\t3\tw\tt\tcmd\t/p\t0\t0", "s:0.0").unwrap();
        if let Action::Tmux(cmd) = pane_select_action(&p) {
            assert!(cmd.contains(
                "select-pane -t 's:2.3' \\; select-window -t 's:2' \\; switch-client -t 's'"
            ));
        } else {
            panic!("expected tmux action");
        }
    }
}
