//! `find-pane` palette — tree of sessions/windows/panes — port of
//! `src/palettes/find-pane.ts`.
//!
//! The tree sits in the left column; a preview panel on the right shows the
//! highlighted pane's live screen, so you can tell two `bash` panes apart
//! without switching to them.
//!
//! Panes can be moved from here too: Alt+m picks the highlighted pane up, and
//! the next key drops it relative to whichever pane is highlighted then — see
//! [`PaneMover`].
//!
//! Each pane's row also says whether it shows output you haven't seen (a `•`
//! by its index, a count on its session) and what an AI agent in it is doing
//! (`working`, `waiting`) — see [`PaneChanges`].

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use super::pane_changes::{AgentState, PaneChanges, PaneInfo};
use crate::ansi::{parse_screen, render_row, Cell};
use crate::fuzzy::multi_fuzzy_score;
use crate::render::{render_default_item, render_item_styled};
use crate::tmux::{tmux, tmux_quote, tmux_raw};
use crate::types::{
    Action, Colors, Item, ItemsSource, KeyHandler, KeyOutcome, PaletteDef, PreviewCtx,
    RenderItemCtx,
};

const SPINNER: &[char] = &[
    '*', '✳', '⠂', '⠐', '⠁', '⠉', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏',
];

/// Marker color for a pane that is active in its window (a fixed "healthy"
/// green, theme-independent), shared by the Find Pane tree and the inline rows.
const ACTIVE_PANE_FG: &str = "\x1b[38;2;166;227;161m";

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
    /// tmux's `%N` id: unlike `target`, it names the same pane however panes
    /// are renumbered, and session names can't make it ambiguous.
    pub id: String,
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

const PANE_FORMAT: &str = "#{session_name}\t#{window_index}\t#{pane_index}\t#{window_name}\t#{pane_title}\t#{pane_current_command}\t#{pane_current_path}\t#{pane_active}\t#{window_active}\t#{pane_id}";

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
    let id = f.get(9).copied().unwrap_or("");

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
        id: id.to_string(),
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

fn build_items(panes: Vec<Pane>, current_session: &str) -> Vec<Item> {
    let grouped = group_panes(panes);

    let mut items = Vec::new();
    for (session, windows) in &grouped {
        let all_in_session: Vec<Pane> = windows
            .iter()
            .flat_map(|(_, w)| w.panes.iter().cloned())
            .collect();
        items.push(session_item(session, &all_in_session, current_session));
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

/// Marker state for an inline pane row, carried in `Item.data` so the main
/// palette's renderer can tint the glyph using the live theme colors.
#[derive(Clone, Copy)]
struct InlinePane {
    is_current: bool,
    pane_active: bool,
}

/// Marker glyph for an inline pane (color is applied at render time from the
/// theme — see [`render_commands_item`]).
fn pane_inline_glyph(p: &Pane) -> &'static str {
    if p.is_current {
        "▶"
    } else if p.pane_active {
        "●"
    } else {
        "○"
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
        .map(|p| Item {
            icon: Some(pane_inline_glyph(p).to_string()),
            title: p.pane_title.clone(),
            description: Some(pane_inline_description(p)),
            action: pane_select_action(p),
            query_only: true,
            data: Some(Rc::new(InlinePane {
                is_current: p.is_current,
                pane_active: p.pane_active,
            })),
            ..Default::default()
        })
        .collect()
}

/// Renderer for the main `commands` palette: inline panes get a theme-aware
/// marker color (current → accent, active → green, otherwise muted), matching
/// the Find Pane tree; everything else uses the default item renderer.
pub fn render_commands_item(item: &Item, ctx: &RenderItemCtx) -> String {
    if let Some(s) = item
        .data
        .as_ref()
        .and_then(|d| d.downcast_ref::<InlinePane>())
    {
        let color = if s.is_current {
            &ctx.colors.accent
        } else if s.pane_active {
            ACTIVE_PANE_FG
        } else {
            &ctx.colors.muted
        };
        render_item_styled(item, ctx.colors, ctx.active, ctx.width, Some(color))
    } else {
        render_default_item(item, ctx.colors, ctx.active, ctx.width)
    }
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
    changed: usize,
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
    // How many of its panes show output you haven't seen.
    let changed = if changed == 0 {
        String::new()
    } else {
        format!(
            "{} · {}{}{}{} new{}{}",
            colors.muted, colors.reset, row_bg, colors.accent, changed, colors.reset, row_bg
        )
    };
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
    format!("{}{}{}{}{}", marker, name, count, changed, path)
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

fn pane_marker(p: &Pane, picked: bool, colors: &Colors) -> (String, char) {
    if picked {
        (colors.accent.clone(), PICKED_MARKER)
    } else if p.is_current {
        (colors.accent.clone(), '▶')
    } else if p.pane_active {
        (ACTIVE_PANE_FG.to_string(), '●')
    } else {
        (colors.muted.clone(), '○')
    }
}

/// Marker for the pane that is picked up, waiting to be dropped.
const PICKED_MARKER: char = '◆';
/// Tag after the picked-up pane's title.
const PICKED_TAG: &str = "moving";
/// Beside the index of a pane that shows output you haven't seen.
const CHANGED_MARK: char = '•';
/// Tags after an agent's name, by what it's doing (`Done` has none: whether
/// you've seen what it left is the `•`'s to say).
const WORKING_TAG: &str = "working";
const WAITING_TAG: &str = "waiting";

/// What a pane's row says about it besides its name.
#[derive(Clone, Copy, Default)]
struct PaneMarks {
    /// Picked up, to be moved.
    picked: bool,
    /// Shows output you haven't seen.
    changed: bool,
    /// What the agent in it is doing.
    state: Option<AgentState>,
}

fn render_pane(
    p: &Pane,
    tree_prefix: &str,
    colors: &Colors,
    row_bg: &str,
    active: bool,
    marks: PaneMarks,
    width: i64,
) -> String {
    let picked = marks.picked;
    let (marker_color, marker_char) = pane_marker(p, picked, colors);
    let title_style = if active || picked {
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
    // Waiting on you is the one to act on, so it stands out; working doesn't.
    let state = match marks.state {
        Some(AgentState::Working) => Some((colors.muted.clone(), WORKING_TAG)),
        Some(AgentState::Waiting) => {
            Some((format!("{}{}", colors.accent, colors.bold), WAITING_TAG))
        }
        Some(AgentState::Done) | None => None,
    };
    if let Some((style, tag)) = state {
        left += &format!("  {}{}{}{}", style, tag, colors.reset, row_bg);
        left_plain_w += 2 + clen(tag);
    }
    if picked {
        left += &format!(
            "  {}{}{}{}{}",
            colors.accent, colors.bold, PICKED_TAG, colors.reset, row_bg
        );
        left_plain_w += 2 + clen(PICKED_TAG);
    }

    let index = format!("{}.{}", p.window_index, p.pane_index);
    let mut right = format!("{}{}{}{}", colors.muted, index, colors.reset, row_bg);
    let mut right_plain_w = clen(&index);
    if marks.changed {
        right = format!(
            "{}{}{}{} {}",
            colors.accent, CHANGED_MARK, colors.reset, row_bg, right
        );
        right_plain_w += 2;
    }
    let gap = (width - left_plain_w - right_plain_w).max(1);
    format!("{}{}{}", left, " ".repeat(gap as usize), right)
}

fn render_item_impl(item: &Item, ctx: &RenderItemCtx, state: &FindPane) -> String {
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
        } => render_session(
            session,
            *count,
            state.changes.changed_in(session),
            path,
            *is_current,
            colors,
            row_bg,
        ),
        ItemData::Window {
            window_name,
            tree_prefix,
            ..
        } => render_window(window_name, tree_prefix, colors, row_bg, active),
        ItemData::Pane { pane, tree_prefix } => {
            let id = pane_ref(pane);
            let marks = PaneMarks {
                picked: state.mover.is_picked(pane),
                changed: state.changes.changed(id),
                state: state.changes.state(id),
            };
            render_pane(pane, tree_prefix, colors, row_bg, active, marks, ctx.width)
        }
    }
}

// ---- moving a pane: pick it up, drop it elsewhere ---------------------------

/// Alt+m: pick the highlighted pane up (again on it: put it back down).
const KEY_PICK: &str = "\x1bm";
/// Alt+h: drop it beside the highlighted pane.
const KEY_BESIDE: &str = "\x1bh";
/// Alt+s: swap it with the highlighted pane.
const KEY_SWAP: &str = "\x1bs";
/// Alt+n: drop it in a new window, in the highlighted pane's session.
const KEY_NEW_WINDOW: &str = "\x1bn";

/// Where a picked-up pane is dropped, relative to the highlighted one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Placement {
    /// Split the highlighted pane and take the bottom half (Enter).
    Below,
    /// Split it side by side and take the right half (Alt+h).
    Beside,
    /// Trade places with it (Alt+s).
    Swap,
    /// Break out into a new window in its session (Alt+n).
    NewWindow,
}

/// How tmux is told which pane: its `%N` id when the listing had one.
fn pane_ref(p: &Pane) -> &str {
    if p.id.is_empty() {
        &p.target
    } else {
        &p.id
    }
}

fn same_pane(a: &Pane, b: &Pane) -> bool {
    pane_ref(a) == pane_ref(b)
}

/// The tmux command that drops `src` at `dst`, or `None` when it would be
/// dropped onto itself. `-d` everywhere: the client stays on what it was
/// showing, and moving a pane never steals the focus.
fn drop_action(src: &Pane, dst: &Pane, how: Placement) -> Option<Action> {
    let s = tmux_quote(pane_ref(src));
    let d = tmux_quote(pane_ref(dst));
    let cmd = match how {
        // Any pane may go to a new window, its own session's included.
        Placement::NewWindow => format!(
            "break-pane -d -s {} -t {}",
            s,
            tmux_quote(&format!("{}:", dst.session))
        ),
        _ if same_pane(src, dst) => return None,
        Placement::Below => format!("join-pane -d -s {} -t {}", s, d),
        Placement::Beside => format!("join-pane -d -h -s {} -t {}", s, d),
        Placement::Swap => format!("swap-pane -d -s {} -t {}", s, d),
    };
    Some(Action::Tmux(cmd))
}

fn pane_of(item: Option<&Item>) -> Option<&Pane> {
    match item?.data.as_ref()?.downcast_ref::<ItemData>()? {
        ItemData::Pane { pane, .. } => Some(pane),
        _ => None,
    }
}

/// Find Pane's pick-up-and-drop. Alt+m picks the highlighted pane up and
/// clears the search, so the next one finds the destination; then Enter,
/// Alt+h, Alt+s or Alt+n drop it relative to the pane highlighted by then
/// (see [`Placement`]), and Esc puts it back. Until a drop the tree works as
/// usual, live preview included, so you can look before you drop.
#[derive(Default)]
pub struct PaneMover {
    picked: RefCell<Option<Pane>>,
}

impl PaneMover {
    fn picked(&self) -> Option<Pane> {
        self.picked.borrow().clone()
    }

    fn is_picked(&self, p: &Pane) -> bool {
        self.picked
            .borrow()
            .as_ref()
            .is_some_and(|src| same_pane(src, p))
    }

    fn pick(&self, p: Option<&Pane>) {
        self.picked.replace(p.cloned());
    }
}

impl KeyHandler for PaneMover {
    fn on_key(&self, key: &str, highlighted: Option<&Item>) -> KeyOutcome {
        let dst = pane_of(highlighted);
        let Some(src) = self.picked() else {
            // Nothing picked up: Alt+m is the only key of ours.
            if key == KEY_PICK && dst.is_some() {
                self.pick(dst);
                return KeyOutcome::ClearSearch;
            }
            return KeyOutcome::Pass;
        };
        let how = match key {
            "\r" => Placement::Below,
            KEY_BESIDE => Placement::Beside,
            KEY_SWAP => Placement::Swap,
            KEY_NEW_WINDOW => Placement::NewWindow,
            // Alt+m on another pane picks that one up instead.
            KEY_PICK if dst.is_some_and(|d| !same_pane(d, &src)) => {
                self.pick(dst);
                return KeyOutcome::ClearSearch;
            }
            // On the picked one (or nothing), or Esc: put it back down.
            KEY_PICK | "\x1b" => {
                self.pick(None);
                return KeyOutcome::Redraw;
            }
            _ => return KeyOutcome::Pass,
        };
        match dst.and_then(|d| drop_action(&src, d, how)) {
            Some(action) => KeyOutcome::Run(action),
            // Onto itself, or with nothing highlighted: stay picked up.
            None => KeyOutcome::Redraw,
        }
    }

    fn title(&self) -> Option<String> {
        self.picked()
            .map(|p| format!("Move {} ({}) to\u{2026}", p.pane_title, p.target))
    }

    fn footer(&self, selectable: usize) -> Option<String> {
        if self.picked.borrow().is_some() {
            return Some(
                "enter below   alt+h beside   alt+s swap   alt+n new window   esc cancel"
                    .to_string(),
            );
        }
        if selectable == 0 {
            return None; // the palette's "No panes"
        }
        let noun = if selectable == 1 { "pane" } else { "panes" };
        Some(format!("enter jump   alt+m move   {} {}", selectable, noun))
    }

    fn save(&self) -> String {
        self.picked()
            .map(|p| pane_ref(&p).to_string())
            .unwrap_or_default()
    }

    /// Look the pane up again by id: the reopened popup lists panes afresh, and
    /// a pane that went away meanwhile is simply not picked up any more.
    fn restore(&self, state: &str) {
        let line = tmux(&["display-message", "-p", "-t", state, PANE_FORMAT]);
        self.pick(parse_pane_line(&line, "").as_ref());
    }
}

// ---- Find Pane's state ---------------------------------------------------------

/// Find Pane's state, shared by its rows, preview and keys: the pane being
/// moved, and which panes changed since you saw them.
#[derive(Default)]
struct FindPane {
    mover: PaneMover,
    changes: PaneChanges,
}

impl KeyHandler for FindPane {
    fn on_key(&self, key: &str, highlighted: Option<&Item>) -> KeyOutcome {
        // Any key may move off the previewed pane, or close the popup.
        self.changes.settle();
        let outcome = self.mover.on_key(key, highlighted);
        // Enter (or a click) on a pane jumps there, so it's been seen.
        if key == "\r" && matches!(outcome, KeyOutcome::Pass) {
            if let Some(p) = pane_of(highlighted) {
                self.changes.mark_seen(pane_ref(p));
            }
        }
        outcome
    }

    fn title(&self) -> Option<String> {
        self.mover.title()
    }

    fn footer(&self, selectable: usize) -> Option<String> {
        self.mover.footer(selectable)
    }

    fn save(&self) -> String {
        self.mover.save()
    }

    fn restore(&self, state: &str) {
        self.mover.restore(state)
    }
}

/// The panes to watch for changes. Those in the window you were looking at
/// when the palette opened count as seen.
fn watch_list(panes: &[Pane]) -> Vec<PaneInfo> {
    let here = panes
        .iter()
        .find(|p| p.is_current)
        .map(|p| (&p.session, &p.window_index));
    panes
        .iter()
        .map(|p| PaneInfo {
            id: pane_ref(p).to_string(),
            session: p.session.clone(),
            agent: p.agent.clone(),
            on_screen: here == Some((&p.session, &p.window_index)),
        })
        .collect()
}

// ---- preview panel -----------------------------------------------------------

/// Preview lines spent on the pane's identity before its screen starts: target
/// + command, path, rule.
const PREVIEW_CHROME: i64 = 3;

/// The pane's visible screen, one line per pane row, with its colors and
/// attributes as SGR escapes (`-e`). `-N` keeps trailing spaces, which carry a
/// row's background out to the edge; `parse_screen` trims the plain ones.
fn capture_pane(target: &str) -> String {
    tmux_raw(&["capture-pane", "-p", "-e", "-N", "-t", target])
}

/// The bottom `height` rows of a parsed screen, trailing blank rows dropped.
/// The bottom is where the prompt and the newest output are, which identifies a
/// pane far better than its top — an idle shell is all blank up there.
fn screen_tail(mut rows: Vec<Vec<Cell>>, height: i64) -> Vec<Vec<Cell>> {
    while rows.last().is_some_and(|r| r.is_empty()) {
        rows.pop();
    }
    let skip = (rows.len() as i64 - height.max(0)).max(0) as usize;
    rows.split_off(skip)
}

fn preview_lines(p: &Pane, capture: &str, ctx: &PreviewCtx) -> Vec<String> {
    let c = ctx.colors;
    let bg = &c.panel;
    // Every line restores the panel background, per the `PreviewFn` contract.
    let mut out = vec![
        format!(
            "{}{}{}{}{}  {}{}{}{}",
            c.accent, c.bold, p.target, c.reset, bg, c.muted, p.command, c.reset, bg
        ),
        format!("{}{}{}{}", c.muted, shorten_path(&p.path), c.reset, bg),
        format!(
            "{}{}{}{}",
            c.muted,
            "─".repeat(ctx.width.max(0) as usize),
            c.reset,
            bg
        ),
    ];
    // The screen in its own colors; its default fg/bg draw as the theme's, so
    // ordinary text sits in the panel like the rest of the popup.
    for row in screen_tail(parse_screen(capture), ctx.height - PREVIEW_CHROME) {
        out.push(format!(
            "{}{}{}",
            render_row(&row, ctx.width, &c.fg, bg),
            c.reset,
            bg
        ));
    }
    out
}

/// Right-hand panel for the highlighted row. Sessions and windows are not
/// selectable in this tree, so in practice the highlight is always a pane.
fn preview_impl(item: Option<&Item>, ctx: &PreviewCtx) -> Vec<String> {
    let Some(ItemData::Pane { pane, .. }) = item
        .and_then(|i| i.data.as_ref())
        .and_then(|d| d.downcast_ref::<ItemData>())
    else {
        return Vec::new();
    };
    preview_lines(pane, &capture_pane(&pane.target), ctx)
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
    let state = Rc::new(FindPane::default());
    let (listed, rows, looking, ticking) =
        (state.clone(), state.clone(), state.clone(), state.clone());
    PaletteDef {
        title: Some("Find Pane".to_string()),
        grouped: Some(false),
        empty_text: Some("No panes".to_string()),
        items: ItemsSource::Dynamic(Rc::new(move || {
            let Fetched {
                panes,
                current_session,
            } = fetch_panes();
            listed.changes.track(watch_list(&panes));
            build_items(panes, &current_session)
        })),
        render_item: Some(Rc::new(move |item: &Item, ctx: &RenderItemCtx| {
            render_item_impl(item, ctx, &rows)
        })),
        filter: Some(Rc::new(filter_tree)),
        initial_selected: Some(Rc::new(initial_selected)),
        preview: Some(Rc::new(move |item: Option<&Item>, ctx: &PreviewCtx| {
            // The preview's pane is the one you're looking at.
            looking.changes.viewing(pane_of(item).map(pane_ref));
            preview_impl(item, ctx)
        })),
        before_frame: Some(Rc::new(move || ticking.changes.refresh())),
        keys: Some(state),
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
        // Marker state rides in `data` (color is applied from the live theme).
        let s0 = items[0].data.as_ref().unwrap().downcast_ref::<InlinePane>().unwrap();
        assert!(s0.is_current && s0.pane_active);

        // Inactive pane: hollow marker; not current/active.
        assert_eq!(items[1].icon.as_deref(), Some("○"));
        let s1 = items[1].data.as_ref().unwrap().downcast_ref::<InlinePane>().unwrap();
        assert!(!s1.is_current && !s1.pane_active);
    }

    #[test]
    fn inline_marker_color_follows_theme() {
        let colors = Colors {
            accent: "ACCENT".into(),
            muted: "MUTED".into(),
            ..Default::default()
        };
        let mk_ctx = || RenderItemCtx {
            colors: &colors,
            active: false,
            width: 60,
        };
        let cur = parse_pane_line("s\t0\t0\tw\tt\tcmd\t/p\t1\t1", "s:0.0").unwrap();
        let other = parse_pane_line("s\t0\t1\tw\tt\tcmd\t/p\t0\t0", "s:0.0").unwrap();
        let items = panes_to_inline_items(&[cur, other]);

        // Current pane marker uses the theme accent.
        let row_cur = render_commands_item(&items[0], &mk_ctx());
        assert!(row_cur.contains("ACCENT"));
        // Inactive pane marker uses muted, never accent.
        let row_other = render_commands_item(&items[1], &mk_ctx());
        assert!(row_other.contains("MUTED"));
        assert!(!row_other.contains("ACCENT"));
    }

    fn preview_colors() -> Colors {
        Colors {
            fg: "FG".into(),
            muted: "MUT".into(),
            accent: "ACC".into(),
            panel: "PANEL".into(),
            reset: "RESET".into(),
            ..Default::default()
        }
    }

    #[test]
    fn screen_tail_drops_trailing_blanks_and_keeps_the_bottom() {
        let tail = |capture: &str, height: i64| -> Vec<String> {
            screen_tail(parse_screen(capture), height)
                .iter()
                .map(|row| row.iter().map(|(c, _)| c).collect())
                .collect()
        };
        // An idle shell: prompt near the top, blank rows below it.
        assert_eq!(tail("$ ls\nfoo\n$ \n\n\n", 4), vec!["$ ls", "foo", "$"]);
        // A full screen: the newest output is at the bottom, so that is what shows.
        assert_eq!(tail("a\nb\nc\nd", 2), vec!["c", "d"]);
        // Degenerate heights never panic or over-take.
        assert!(tail("a\nb", 0).is_empty());
        assert!(tail("a\nb", -3).is_empty());
        assert!(tail("\n\n", 5).is_empty());
        // The spaces `-N` pads every row with are not content.
        assert_eq!(tail("$ ls      \n          \n", 5), vec!["$ ls"]);
    }

    #[test]
    fn preview_keeps_the_screen_colors_within_the_column() {
        let p = parse_pane_line("main\t1\t0\teditor\tvim\tvim\t/tmp\t1\t1", "main:1.0").unwrap();
        let colors = Colors {
            fg: "\x1b[38;5;7m".into(),
            panel: "\x1b[48;5;0m".into(),
            reset: "\x1b[0m".into(),
            ..Default::default()
        };
        let ctx = PreviewCtx {
            colors: &colors,
            width: 8,
            height: 6,
        };
        // A red word, a blue bar running past the column's edge, plain text.
        let capture = "\x1b[31mred\x1b[39m\n\x1b[44mstatus bar here\x1b[0m\nplain\n";
        let lines = preview_lines(&p, capture, &ctx);
        let screen = &lines[PREVIEW_CHROME as usize..];
        assert_eq!(screen.len(), 3);
        assert!(screen[0].contains("\x1b[31mred"));
        assert!(screen[1].contains("\x1b[44mstatus b"));
        // Default text takes the theme's fg on the panel, like the rest of the popup.
        assert!(screen[2].contains("\x1b[0m\x1b[38;5;7m\x1b[48;5;0mplain"));
        // Cut to the column, never past it.
        let widths: Vec<i64> = screen
            .iter()
            .map(|l| crate::text::display_width(l))
            .collect();
        assert_eq!(widths, vec![3, 8, 5]);
    }

    #[test]
    fn preview_leads_with_the_pane_identity_then_its_screen() {
        let p = parse_pane_line(
            "main\t1\t0\teditor\tnvim\tnvim\t/tmp/proj\t1\t1",
            "main:1.0",
        )
        .unwrap();
        let colors = preview_colors();
        let ctx = PreviewCtx {
            colors: &colors,
            width: 20,
            height: 6,
        };
        let lines = preview_lines(&p, "one\ntwo\nthree\n\n\n", &ctx);

        // target + command, path, rule, then `height - chrome` content rows.
        assert!(lines[0].contains("main:1.0"));
        assert!(lines[0].contains("nvim"));
        assert!(lines[1].contains("/tmp/proj"));
        // The rule spans the column exactly (`─` is one cell wide).
        assert_eq!(lines[2].matches('─').count(), 20);
        assert_eq!(lines.len(), PREVIEW_CHROME as usize + 3);
        assert!(lines[3].contains("one"));
        assert!(lines[5].contains("three"));

        // Every line restores the panel background for the padding that follows.
        for line in &lines {
            assert!(
                line.ends_with("PANEL"),
                "line does not restore panel: {line:?}"
            );
        }
    }

    #[test]
    fn preview_is_empty_for_a_non_pane_row() {
        let colors = preview_colors();
        let ctx = PreviewCtx {
            colors: &colors,
            width: 20,
            height: 10,
        };
        let session = session_item("main", &[], "main");
        assert!(preview_impl(Some(&session), &ctx).is_empty());
        assert!(preview_impl(None, &ctx).is_empty());
    }

    /// `name` at `target` with tmux id `id`, as `list-panes` would list it.
    fn listed(target: &str, id: &str, title: &str) -> Pane {
        let (session, rest) = target.split_once(':').unwrap();
        let (window, pane) = rest.split_once('.').unwrap();
        let line = format!("{session}\t{window}\t{pane}\tw\t{title}\tcmd\t/p\t0\t0\t{id}");
        parse_pane_line(&line, "").unwrap()
    }

    fn run_cmd(outcome: KeyOutcome) -> String {
        match outcome {
            KeyOutcome::Run(Action::Tmux(cmd)) => cmd,
            _ => panic!("expected a tmux command to run"),
        }
    }

    #[test]
    fn parses_the_pane_id() {
        let p = listed("main:1.0", "%7", "nvim");
        assert_eq!((p.id.as_str(), pane_ref(&p)), ("%7", "%7"));
        // A listing without one (an older format) falls back to the target.
        let old = parse_pane_line("main\t1\t0\tw\tt\tc\t/p\t0\t0", "").unwrap();
        assert_eq!(pane_ref(&old), "main:1.0");
    }

    #[test]
    fn drops_name_panes_by_id() {
        let src = listed("main:1.0", "%1", "nvim");
        let dst = listed("work:2.1", "%9", "logs");
        let cmd = |how| match drop_action(&src, &dst, how) {
            Some(Action::Tmux(c)) => c,
            _ => panic!("expected a tmux command"),
        };
        assert_eq!(cmd(Placement::Below), "join-pane -d -s '%1' -t '%9'");
        assert_eq!(cmd(Placement::Beside), "join-pane -d -h -s '%1' -t '%9'");
        assert_eq!(cmd(Placement::Swap), "swap-pane -d -s '%1' -t '%9'");
        // A new window goes in the highlighted pane's session.
        assert_eq!(
            cmd(Placement::NewWindow),
            "break-pane -d -s '%1' -t 'work:'"
        );
    }

    #[test]
    fn a_pane_is_never_dropped_onto_itself() {
        let p = listed("main:1.0", "%1", "nvim");
        for how in [Placement::Below, Placement::Beside, Placement::Swap] {
            assert!(drop_action(&p, &p, how).is_none(), "{how:?}");
        }
        // ...though it can still break out into a window of its own.
        assert!(drop_action(&p, &p, Placement::NewWindow).is_some());
    }

    #[test]
    fn pick_up_then_drop() {
        let mover = PaneMover::default();
        let src = pane_item(&listed("main:1.0", "%1", "nvim"), String::new());
        let dst = pane_item(&listed("work:2.1", "%9", "logs"), String::new());

        // Nothing picked up: Enter and typing are the runner's; Alt+m picks the
        // highlighted pane up and starts a fresh search for the destination.
        assert!(matches!(mover.on_key("\r", Some(&src)), KeyOutcome::Pass));
        assert!(matches!(mover.on_key("x", Some(&src)), KeyOutcome::Pass));
        assert!(matches!(mover.on_key(KEY_PICK, None), KeyOutcome::Pass));
        assert!(matches!(
            mover.on_key(KEY_PICK, Some(&src)),
            KeyOutcome::ClearSearch
        ));
        assert_eq!(mover.save(), "%1");
        assert_eq!(
            mover.title().as_deref(),
            Some("Move nvim (main:1.0) to\u{2026}")
        );

        // Picked up: typing still searches, and dropping it onto itself does
        // nothing but keep it picked up.
        assert!(matches!(mover.on_key("l", Some(&dst)), KeyOutcome::Pass));
        assert!(matches!(mover.on_key("\r", Some(&src)), KeyOutcome::Redraw));
        assert!(matches!(mover.on_key("\r", None), KeyOutcome::Redraw));
        assert_eq!(mover.save(), "%1");

        assert_eq!(
            run_cmd(mover.on_key("\r", Some(&dst))),
            "join-pane -d -s '%1' -t '%9'"
        );
        assert_eq!(
            run_cmd(mover.on_key(KEY_BESIDE, Some(&dst))),
            "join-pane -d -h -s '%1' -t '%9'"
        );
        assert_eq!(
            run_cmd(mover.on_key(KEY_SWAP, Some(&dst))),
            "swap-pane -d -s '%1' -t '%9'"
        );
        assert_eq!(
            run_cmd(mover.on_key(KEY_NEW_WINDOW, Some(&dst))),
            "break-pane -d -s '%1' -t 'work:'"
        );
    }

    #[test]
    fn put_back_down_or_pick_another() {
        let mover = PaneMover::default();
        let a = pane_item(&listed("main:1.0", "%1", "a"), String::new());
        let b = pane_item(&listed("main:1.1", "%2", "b"), String::new());

        // Esc puts it back down; the next Esc is the runner's again.
        mover.on_key(KEY_PICK, Some(&a));
        assert!(matches!(mover.on_key("\x1b", Some(&b)), KeyOutcome::Redraw));
        assert_eq!(mover.save(), "");
        assert!(mover.title().is_none());
        assert!(matches!(mover.on_key("\x1b", Some(&b)), KeyOutcome::Pass));

        // Alt+m on another pane swaps which one is picked up...
        mover.on_key(KEY_PICK, Some(&a));
        assert!(matches!(
            mover.on_key(KEY_PICK, Some(&b)),
            KeyOutcome::ClearSearch
        ));
        assert_eq!(mover.save(), "%2");
        // ...and on the picked one puts it down.
        assert!(matches!(
            mover.on_key(KEY_PICK, Some(&b)),
            KeyOutcome::Redraw
        ));
        assert_eq!(mover.save(), "");
    }

    #[test]
    fn footer_says_what_the_keys_do() {
        let mover = PaneMover::default();
        assert_eq!(
            mover.footer(1).as_deref(),
            Some("enter jump   alt+m move   1 pane")
        );
        assert_eq!(
            mover.footer(3).as_deref(),
            Some("enter jump   alt+m move   3 panes")
        );
        assert_eq!(mover.footer(0), None);

        let p = listed("main:1.0", "%1", "nvim");
        mover.pick(Some(&p));
        let hint = mover.footer(0).unwrap();
        for key in [
            "enter below",
            "alt+h beside",
            "alt+s swap",
            "alt+n new window",
            "esc cancel",
        ] {
            assert!(hint.contains(key), "{hint:?} lacks {key:?}");
        }
    }

    #[test]
    fn the_picked_pane_is_marked_in_the_tree() {
        // Real escapes, so the width check below sees only what is drawn.
        let colors = Colors {
            accent: "\x1b[34m".into(),
            muted: "\x1b[90m".into(),
            ..Default::default()
        };
        let ctx = RenderItemCtx {
            colors: &colors,
            active: false,
            width: 50,
        };
        let state = FindPane::default();
        let picked = listed("main:1.0", "%1", "nvim");
        let other = listed("main:1.1", "%2", "logs");
        state.mover.pick(Some(&picked));

        let row = render_item_impl(&pane_item(&picked, "├─ ".into()), &ctx, &state);
        assert!(row.contains(&format!("\x1b[34m{PICKED_MARKER}")));
        assert!(row.contains(PICKED_TAG));
        // The tag counts toward the row width, so `1.0` still ends the row.
        assert_eq!(crate::text::display_width(&row), 50);

        let row = render_item_impl(&pane_item(&other, "└─ ".into()), &ctx, &state);
        assert!(!row.contains(PICKED_MARKER) && !row.contains(PICKED_TAG));
    }

    #[test]
    fn changed_panes_and_agents_are_marked_in_the_tree() {
        let colors = Colors {
            accent: "\x1b[34m".into(),
            muted: "\x1b[90m".into(),
            bold: "\x1b[1m".into(),
            ..Default::default()
        };
        let p = listed("main:1.0", "%1", "claude");
        let row = |marks: PaneMarks| render_pane(&p, "├─ ", &colors, "", false, marks, 50);

        // Unseen output: an accent `•` before the index, which still ends the row.
        let changed = row(PaneMarks {
            changed: true,
            ..Default::default()
        });
        assert!(changed.contains(&format!("\x1b[34m{CHANGED_MARK}")));
        assert!(changed.ends_with("1.0"));
        assert_eq!(crate::text::display_width(&changed), 50);
        assert!(!row(PaneMarks::default()).contains(CHANGED_MARK));

        // A working agent says so quietly; one waiting on you stands out.
        let working = row(PaneMarks {
            state: Some(AgentState::Working),
            ..Default::default()
        });
        assert!(working.contains(&format!("\x1b[90m{WORKING_TAG}")));
        let waiting = row(PaneMarks {
            state: Some(AgentState::Waiting),
            changed: true,
            ..Default::default()
        });
        assert!(waiting.contains(&format!("\x1b[34m\x1b[1m{WAITING_TAG}")));
        assert_eq!(crate::text::display_width(&waiting), 50);
        // Done has no tag: the `•` says whether there's anything to read.
        let done = row(PaneMarks {
            state: Some(AgentState::Done),
            ..Default::default()
        });
        assert!(!done.contains(WORKING_TAG) && !done.contains(WAITING_TAG));
    }

    #[test]
    fn sessions_count_their_changed_panes() {
        let colors = Colors {
            accent: "ACC".into(),
            ..Default::default()
        };
        let row = render_session("main", 4, 2, "/tmp", false, &colors, "");
        assert!(row.contains("ACC2 new"));
        let row = render_session("main", 4, 0, "/tmp", false, &colors, "");
        assert!(!row.contains("new"));
    }

    #[test]
    fn the_window_you_were_looking_at_counts_as_seen() {
        let line = |target: &str, id: &str| {
            let (session, rest) = target.split_once(':').unwrap();
            let (window, pane) = rest.split_once('.').unwrap();
            format!("{session}\t{window}\t{pane}\tw\tt\tclaude\t/p\t0\t0\t{id}")
        };
        let panes: Vec<Pane> = [
            ("main:1.0", "%1"),
            ("main:1.1", "%2"),
            ("main:2.0", "%3"),
            ("work:1.0", "%4"),
        ]
        .iter()
        .map(|(t, id)| parse_pane_line(&line(t, id), "main:1.1").unwrap())
        .collect();
        let watched = watch_list(&panes);
        let on_screen: Vec<&str> = watched
            .iter()
            .filter(|w| w.on_screen)
            .map(|w| w.id.as_str())
            .collect();
        // The current pane's window only: not another window, nor another
        // session's window with the same index.
        assert_eq!(on_screen, vec!["%1", "%2"]);
        assert_eq!(watched[3].session, "work");
        assert_eq!(watched[0].agent, "claude");
    }

    #[test]
    fn find_pane_keys_still_move_panes() {
        let state = FindPane::default();
        let src = pane_item(&listed("main:1.0", "%1", "nvim"), String::new());
        // Enter on a pane is still the runner's jump.
        assert!(matches!(state.on_key("\r", Some(&src)), KeyOutcome::Pass));
        assert!(matches!(
            state.on_key(KEY_PICK, Some(&src)),
            KeyOutcome::ClearSearch
        ));
        assert_eq!(state.save(), "%1");
        assert!(state.title().is_some_and(|t| t.starts_with("Move nvim")));
        assert!(state.footer(0).is_some_and(|f| f.contains("esc cancel")));
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
