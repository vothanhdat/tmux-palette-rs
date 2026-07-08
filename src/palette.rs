//! Interactive palette runner — port of `src/palette.ts`.
//!
//! Owns the raw-mode render/input loop: search editing (with selection), arrow/
//! mouse navigation, in-process palette navigation (a Raycast-style back stack),
//! and dispatch of the chosen action to the launcher via the cmd file.

use std::process;
use std::rc::Rc;

use crate::dispatch::dispatch_to_file;
use crate::fuzzy::default_filter;
use crate::raw::{self, is_interactive, read_stdin, terminal_size, write_stdout, RawMode};
use crate::render::{
    build_rows, compose_footer, compose_header, compose_list_body, compose_search,
    first_selectable, is_selectable, render_category, render_default_item, step, Row, RowAction,
};
use crate::theme::{cursor_tint, make_colors, popup_flags, resolve_active_theme};
use crate::types::{
    Action, ActionContext, Colors, Item, PaletteDef, PopupAction, RenderItemCtx, Theme,
};
use crate::user_config::{user_aliases, user_shortcuts, user_sizing};

pub type PaletteLoader = Rc<dyn Fn(&str) -> Option<PaletteDef>>;

struct NavState {
    def: PaletteDef,
    name: String,
    selected: usize,
    scroll: usize,
    filter: Vec<char>,
    filter_cursor: usize,
    selection_anchor: Option<usize>,
}

#[derive(Clone, Copy)]
struct EscAction {
    y: i64,
    x_start: i64,
    x_end: i64,
}

fn apply_user_overrides(items: Vec<Item>) -> Vec<Item> {
    let shortcuts = user_shortcuts();
    let aliases = user_aliases();
    items
        .into_iter()
        .map(|mut i| {
            if i.shortcut.is_none() {
                i.shortcut = shortcuts.get(&i.title).cloned();
            }
            if let Some(extra) = aliases.get(&i.title) {
                let mut a = i.aliases.take().unwrap_or_default();
                a.extend(extra.clone());
                i.aliases = Some(a);
            }
            i
        })
        .collect()
}

/// Items shown when the search box is empty: everything except `query_only`
/// rows, which only surface once the user starts typing.
fn drop_query_only(items: &[Item]) -> Vec<Item> {
    items.iter().filter(|i| !i.query_only).cloned().collect()
}

fn clamp_scroll(rows: &[Row], list_height: usize, selected: usize, scroll: usize) -> usize {
    let mut scroll = scroll as i64;
    let lh = list_height as i64;
    let selected_row_idx = rows
        .iter()
        .position(|r| matches!(r, Row::Item { item_index, .. } if *item_index == selected));
    if let Some(idx) = selected_row_idx {
        let idx = idx as i64;
        if idx < scroll {
            scroll = idx;
        }
        if idx >= scroll + lh {
            scroll = idx - lh + 1;
        }
    }
    let max_scroll = (rows.len() as i64 - lh).max(0);
    scroll.clamp(0, max_scroll) as usize
}

fn build_footer_text(selectable_count: i64, empty_text: &str) -> String {
    if selectable_count == 0 {
        return empty_text.to_string();
    }
    let noun = if selectable_count == 1 {
        "command"
    } else {
        "commands"
    };
    format!(
        "enter select   up/down move   {} {}",
        selectable_count, noun
    )
}

fn nav_delta(key: &str) -> Option<i64> {
    match key {
        "\x1b[A" | "\x10" => Some(-1),
        "\x1b[B" | "\x0e" => Some(1),
        "\x1b[5~" => Some(-10),
        "\x1b[6~" => Some(10),
        _ => None,
    }
}

fn parse_mouse_event(key: &str) -> Option<(i64, i64, i64, char)> {
    let rest = key.strip_prefix("\x1b[<")?;
    let end = rest.find(['m', 'M'])?;
    let kind = rest.as_bytes()[end] as char;
    let mut it = rest[..end].split(';');
    let button = it.next()?.parse::<i64>().ok()?;
    let x = it.next()?.parse::<i64>().ok()?;
    let y = it.next()?.parse::<i64>().ok()?;
    Some((button, x, y, kind))
}

fn is_ws(c: char) -> bool {
    c.is_whitespace()
}

fn word_back(s: &[char], from: usize) -> usize {
    let mut i = from.min(s.len());
    while i > 0 && is_ws(s[i - 1]) {
        i -= 1;
    }
    while i > 0 && !is_ws(s[i - 1]) {
        i -= 1;
    }
    i
}

fn word_forward(s: &[char], from: usize) -> usize {
    let mut i = from.min(s.len());
    while i < s.len() && is_ws(s[i]) {
        i += 1;
    }
    while i < s.len() && !is_ws(s[i]) {
        i += 1;
    }
    i
}

pub struct Runner {
    current_def: PaletteDef,
    current_name: String,
    theme: Theme,
    colors: Colors,
    items: Vec<Item>,
    title: String,
    grouped: bool,
    empty_text: String,
    cmd_file: Option<String>,

    filter: Vec<char>,
    filter_cursor: usize,
    selection_anchor: Option<usize>,
    selected: usize,
    scroll: usize,

    row_actions: Vec<RowAction>,
    esc_action: Option<EscAction>,
    stack: Vec<NavState>,

    loader: Option<PaletteLoader>,
    raw_mode: Option<RawMode>,
}

impl Runner {
    fn new(def: PaletteDef, loader: Option<PaletteLoader>, initial_name: &str) -> Runner {
        let theme = resolve_active_theme(&def.theme);
        let colors = make_colors(&theme);
        let items = apply_user_overrides(def.resolve_items());
        let title = def.title.clone().unwrap_or_else(|| "Commands".to_string());
        let grouped = def.grouped != Some(false);
        let empty_text = def
            .empty_text
            .clone()
            .unwrap_or_else(|| "No results".to_string());
        let selected = match &def.initial_selected {
            Some(f) => f(&items).max(0) as usize,
            None => 0,
        };
        Runner {
            current_def: def,
            current_name: initial_name.to_string(),
            theme,
            colors,
            items,
            title,
            grouped,
            empty_text,
            cmd_file: std::env::var("TMUX_PALETTE_CMD").ok(),
            filter: Vec::new(),
            filter_cursor: 0,
            selection_anchor: None,
            selected,
            scroll: 0,
            row_actions: Vec::new(),
            esc_action: None,
            stack: Vec::new(),
            loader,
            raw_mode: None,
        }
    }

    fn ctx(&self) -> ActionContext {
        ActionContext {
            cmd_file: self.cmd_file.clone(),
        }
    }

    fn load_def(&mut self, d: PaletteDef) {
        self.theme = resolve_active_theme(&d.theme);
        self.colors = make_colors(&self.theme);
        self.items = apply_user_overrides(d.resolve_items());
        self.title = d.title.clone().unwrap_or_else(|| "Commands".to_string());
        self.grouped = d.grouped != Some(false);
        self.empty_text = d
            .empty_text
            .clone()
            .unwrap_or_else(|| "No results".to_string());
        self.current_def = d;
    }

    fn navigate_to(&mut self, name: &str) {
        let Some(loader) = self.loader.clone() else {
            return;
        };
        let Some(next) = loader(name) else {
            return;
        };
        self.stack.push(NavState {
            def: self.current_def.clone(),
            name: self.current_name.clone(),
            selected: self.selected,
            scroll: self.scroll,
            filter: self.filter.clone(),
            filter_cursor: self.filter_cursor,
            selection_anchor: self.selection_anchor,
        });
        self.load_def(next);
        self.current_name = name.to_string();
        self.selected = 0;
        self.scroll = 0;
        self.filter.clear();
        self.filter_cursor = 0;
        self.selection_anchor = None;
        self.render();
    }

    fn navigate_back(&mut self) {
        let Some(prev) = self.stack.pop() else {
            self.exit_now();
        };
        self.load_def(prev.def);
        self.current_name = prev.name;
        self.selected = prev.selected;
        self.scroll = prev.scroll;
        self.filter = prev.filter;
        self.filter_cursor = prev.filter_cursor;
        self.selection_anchor = prev.selection_anchor;
        self.render();
    }

    fn visible(&self) -> Vec<Item> {
        let needle: String = self.filter.iter().collect();
        let needle = needle.trim();
        if needle.is_empty() {
            // `query_only` items (e.g. inlined live panes) stay hidden until the
            // user types, keeping the resting palette uncluttered.
            return drop_query_only(&self.items);
        }
        if let Some(f) = &self.current_def.filter {
            return f(&self.items, needle);
        }
        default_filter(&self.items, needle)
    }

    fn ensure_selectable(&mut self, vis: &[Item]) {
        if is_selectable(vis.get(self.selected)) {
            return;
        }
        let f = first_selectable(vis);
        self.selected = if f >= 0 { f as usize } else { 0 };
    }

    fn render(&mut self) {
        let (width, height) = terminal_size();
        let vis = self.visible();
        self.ensure_selectable(&vis);

        if let Some(on_select) = self.current_def.on_select.clone() {
            if let Some(preview) = on_select(vis.get(self.selected)) {
                self.theme = preview;
                self.colors = make_colors(&self.theme);
            }
        }

        let colors = self.colors.clone();
        let theme = self.theme.clone();
        let filter_str: String = self.filter.iter().collect();
        let rows = build_rows(&vis, self.grouped, !self.filter.is_empty());

        let bordered = std::env::var("TMUX_PALETTE_BORDERED").ok().as_deref() == Some("1");
        let chrome_rows = if bordered { 5 } else { 7 };
        let list_height = (height - chrome_rows).max(1) as usize;
        self.scroll = clamp_scroll(&rows, list_height, self.selected, self.scroll);
        let scroll = self.scroll;

        let pad_x = std::env::var("TMUX_PALETTE_PADX")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(3)
            .max(0);
        let body_width = (width - pad_x * 2).max(1);
        let blank = format!(
            "{}{}{}",
            colors.panel,
            " ".repeat(width.max(0) as usize),
            colors.reset
        );

        let header = compose_header(&self.title, width, pad_x, body_width, &colors);
        self.esc_action = Some(EscAction {
            y: if bordered { 1 } else { 2 },
            x_start: header.esc_x1,
            x_end: header.esc_x2,
        });

        let render_item = self.current_def.render_item.clone();
        let body = {
            let colors_ref = &colors;
            let render_row = |row: &Row, is_selected: bool| -> String {
                let row_bg = if is_selected {
                    &colors_ref.selected
                } else {
                    &colors_ref.panel
                };
                match row {
                    Row::Category { category } => render_category(category, colors_ref, row_bg),
                    Row::Item { item, .. } => match &render_item {
                        Some(ri) => ri(
                            item,
                            &RenderItemCtx {
                                colors: colors_ref,
                                active: is_selected,
                                width: body_width,
                            },
                        ),
                        None => render_default_item(item, colors_ref, is_selected, body_width),
                    },
                }
            };
            compose_list_body(
                &rows,
                scroll,
                list_height,
                self.selected,
                body_width,
                pad_x,
                &colors,
                if bordered { 4 } else { 5 },
                render_row,
            )
        };
        self.row_actions = body.row_actions;

        let selectable_count = vis.iter().filter(|i| is_selectable(Some(i))).count() as i64;
        let footer_text = build_footer_text(selectable_count, &self.empty_text);

        let (sel_start, sel_end) = match self.selection_anchor {
            Some(a) if a != self.filter_cursor => (
                Some(a.min(self.filter_cursor)),
                Some(a.max(self.filter_cursor)),
            ),
            _ => (None, None),
        };

        let mut inner: Vec<String> = Vec::new();
        inner.push(header.line);
        inner.push(compose_search(
            &filter_str,
            pad_x,
            body_width,
            &colors,
            sel_start,
            sel_end,
        ));
        inner.push(blank.clone());
        inner.extend(body.lines);
        inner.push(blank.clone());
        inner.push(compose_footer(&footer_text, pad_x, body_width, &colors));

        let lines: Vec<String> = if bordered {
            inner
        } else {
            let mut v = Vec::with_capacity(inner.len() + 2);
            v.push(blank.clone());
            v.extend(inner);
            v.push(blank);
            v
        };

        let search_row = if bordered { 2 } else { 3 };
        let cursor_col =
            (pad_x + 3 + self.filter_cursor as i64).min(pad_x + 3 + (body_width - 2).max(0));

        let mut out = String::new();
        out.push_str("\x1b[?2026h\x1b[?25l\x1b[H");
        out.push_str(&lines.join("\n"));
        out.push_str(&format!("\x1b[{};{}H\x1b[5 q", search_row, cursor_col));
        out.push_str(&cursor_tint(&theme));
        out.push_str("\x1b[?25h\x1b[?2026l");
        write_stdout(out.as_bytes());
    }

    fn cleanup(&mut self) {
        write_stdout(
            format!(
                "{}\x1b[?1000l\x1b[?1006l\x1b[?25h\x1b[0 q\x1b]112\x07\x1b[2J\x1b[H",
                self.colors.reset
            )
            .as_bytes(),
        );
        if let Some(rm) = &mut self.raw_mode {
            rm.disable();
        }
    }

    fn exit_now(&mut self) -> ! {
        self.cleanup();
        process::exit(0);
    }

    // ---- popup relaunch ----

    fn popup_dim_expr(spec: &str, axis: &str, pad: i64) -> String {
        if let Some(num) = spec.strip_suffix('%') {
            let pct: i64 = num.parse().unwrap_or(80);
            format!(
                "$(( $(tmux display-message -p '#{{{}}}') * {} / 100 - {} ))",
                axis,
                pct,
                2 * pad
            )
        } else {
            let cells = spec.parse::<i64>().unwrap_or(0) - 2 * pad;
            format!("{}", cells.max(1))
        }
    }

    fn build_popup_relaunch_command(&self, action: &PopupAction, relaunch_name: &str) -> String {
        let sizing = user_sizing();
        let pad_x = action.pad_x.or(sizing.popup_pad_x).unwrap_or(0);
        let pad_y = action.pad_y.or(sizing.popup_pad_y).unwrap_or(0);
        let width = action
            .width
            .clone()
            .or(sizing.popup_width)
            .unwrap_or_else(|| "80%".to_string());
        let height = action
            .height
            .clone()
            .or(sizing.popup_height)
            .unwrap_or_else(|| "80%".to_string());
        let w_expr = Self::popup_dim_expr(&width, "client_width", pad_x);
        let h_expr = Self::popup_dim_expr(&height, "client_height", pad_y);
        let bin = std::env::var("TMUX_PALETTE_BIN").unwrap_or_else(|_| "tmux-palette".to_string());
        format!(
            "tmux display-popup -E {} -h {} -w {} {}; tmux run-shell -b '{} {}'",
            popup_flags(&self.theme, action.border.as_deref()),
            h_expr,
            w_expr,
            action.popup,
            bin,
            relaunch_name
        )
    }

    fn dispatch_popup_action(&mut self, action: &PopupAction) -> ! {
        self.cleanup();
        if let Some(file) = self.cmd_file.clone() {
            let cmd = self.build_popup_relaunch_command(action, &self.current_name.clone());
            let _ = std::fs::write(file, format!("shell:{}", cmd));
        }
        process::exit(0);
    }

    fn dispatch_direct_action(&mut self, item: &Item) -> ! {
        self.cleanup();
        if let Action::Run(f) = &item.action {
            f(&self.ctx());
            process::exit(0);
        }
        dispatch_to_file(&item.action, self.cmd_file.as_deref());
        process::exit(0);
    }

    fn dispatch_apply_action(&mut self, f: &crate::types::RunFn) {
        f(&self.ctx());
        if !self.stack.is_empty() {
            self.navigate_back();
        } else {
            self.exit_now();
        }
    }

    fn activate(&mut self, item: &Item) {
        match &item.action {
            Action::Palette(name) if self.loader.is_some() => {
                let name = name.clone();
                self.navigate_to(&name);
            }
            Action::Apply(f) => {
                let f = f.clone();
                self.dispatch_apply_action(&f);
            }
            Action::Popup(p) => {
                let p = p.clone();
                self.dispatch_popup_action(&p);
            }
            Action::Fill(text) => {
                let text = text.clone();
                self.set_input(&text);
                self.render();
            }
            _ => self.dispatch_direct_action(item),
        }
    }

    /// Replace the search input (used by `Action::Fill` for completion),
    /// resetting the selection to the top of the refreshed results.
    fn set_input(&mut self, text: &str) {
        self.filter = text.chars().collect();
        self.filter_cursor = self.filter.len();
        self.selection_anchor = None;
        self.selected = 0;
        self.scroll = 0;
    }

    fn esc_pressed(&mut self) {
        let esc_mode = user_sizing().esc.unwrap_or_else(|| "back".to_string());
        if esc_mode == "back" && !self.stack.is_empty() {
            self.navigate_back();
            return;
        }
        self.exit_now();
    }

    fn esc_clicked(&self, x: i64, y: i64) -> bool {
        match self.esc_action {
            Some(e) => y == e.y && x >= e.x_start && x <= e.x_end,
            None => false,
        }
    }

    fn handle_row_click(&mut self, y: i64, vis: &[Item]) {
        let Some(hit) = self.row_actions.iter().find(|r| r.y == y).copied() else {
            return;
        };
        let Some(item) = vis.get(hit.item_index) else {
            return;
        };
        if !is_selectable(Some(item)) {
            return;
        }
        self.selected = hit.item_index;
        let item = item.clone();
        self.activate(&item);
    }

    fn handle_mouse_click(&mut self, x: i64, y: i64, vis: &[Item]) {
        if self.esc_clicked(x, y) {
            self.esc_pressed();
            return;
        }
        self.handle_row_click(y, vis);
    }

    fn handle_mouse(&mut self, button: i64, x: i64, y: i64, kind: char, vis: &[Item]) {
        if button == 64 {
            self.selected = step(vis, self.selected, -1);
        } else if button == 65 {
            self.selected = step(vis, self.selected, 1);
        } else if button == 0 && kind == 'M' {
            self.handle_mouse_click(x, y, vis);
        }
        self.render();
    }

    fn handle_navigation_key(&mut self, key: &str, vis: &[Item]) -> bool {
        let Some(delta) = nav_delta(key) else {
            return false;
        };
        let dir = if delta > 0 { 1 } else { -1 };
        let count = delta.unsigned_abs() as usize;
        for _ in 0..count {
            self.selected = step(vis, self.selected, dir);
        }
        true
    }

    fn handle_enter_or_exit(&mut self, key: &str, vis: &[Item]) -> bool {
        if key == "\x1b" {
            if self.selection_anchor.is_some() {
                self.selection_anchor = None;
                self.render();
                return true;
            }
            self.esc_pressed();
            return true;
        }
        if key == "\x03" {
            self.exit_now();
        }
        if key != "\r" {
            return false;
        }
        if let Some(item) = vis.get(self.selected) {
            if is_selectable(Some(item)) {
                let item = item.clone();
                self.activate(&item);
            }
        }
        true
    }

    fn sel_range(&self) -> Option<(usize, usize)> {
        let anchor = self.selection_anchor?;
        let a = anchor.min(self.filter_cursor);
        let b = anchor.max(self.filter_cursor);
        if a == b {
            None
        } else {
            Some((a, b))
        }
    }

    fn delete_selection(&mut self) -> bool {
        match self.sel_range() {
            None => {
                self.selection_anchor = None;
                false
            }
            Some((a, b)) => {
                self.filter.drain(a..b);
                self.filter_cursor = a;
                self.selection_anchor = None;
                true
            }
        }
    }

    fn extend_to(&mut self, to: i64) {
        let target = to.clamp(0, self.filter.len() as i64) as usize;
        if self.selection_anchor.is_none() {
            self.selection_anchor = Some(self.filter_cursor);
        }
        self.filter_cursor = target;
        if self.selection_anchor == Some(self.filter_cursor) {
            self.selection_anchor = None;
        }
    }

    fn collapse_left(&mut self) {
        if let Some((a, _)) = self.sel_range() {
            self.filter_cursor = a;
            self.selection_anchor = None;
        } else {
            self.filter_cursor = self.filter_cursor.saturating_sub(1);
        }
    }

    fn collapse_right(&mut self) {
        if let Some((_, b)) = self.sel_range() {
            self.filter_cursor = b;
            self.selection_anchor = None;
        } else {
            self.filter_cursor = (self.filter_cursor + 1).min(self.filter.len());
        }
    }

    fn handle_edit_key(&mut self, key: &str) -> bool {
        // ---- cursor movement (clears selection on plain moves) ----
        match key {
            "\x1b[D" => {
                self.collapse_left();
                return true;
            }
            "\x1b[C" => {
                self.collapse_right();
                return true;
            }
            "\x1b[H" | "\x01" => {
                self.filter_cursor = 0;
                self.selection_anchor = None;
                return true;
            }
            "\x1b[F" | "\x05" => {
                self.filter_cursor = self.filter.len();
                self.selection_anchor = None;
                return true;
            }
            "\x1bb" | "\x1b[1;3D" | "\x1b[1;5D" | "\x1b\x1b[D" => {
                self.filter_cursor = word_back(&self.filter, self.filter_cursor);
                self.selection_anchor = None;
                return true;
            }
            "\x1bf" | "\x1b[1;3C" | "\x1b[1;5C" | "\x1b\x1b[C" => {
                self.filter_cursor = word_forward(&self.filter, self.filter_cursor);
                self.selection_anchor = None;
                return true;
            }
            // ---- shift + movement: extend selection ----
            "\x1b[1;2D" => {
                self.extend_to(self.filter_cursor as i64 - 1);
                return true;
            }
            "\x1b[1;2C" => {
                self.extend_to(self.filter_cursor as i64 + 1);
                return true;
            }
            "\x1b[1;2H" => {
                self.extend_to(0);
                return true;
            }
            "\x1b[1;2F" => {
                self.extend_to(self.filter.len() as i64);
                return true;
            }
            "\x1b[1;4D" | "\x1b[1;6D" => {
                let t = word_back(&self.filter, self.filter_cursor) as i64;
                self.extend_to(t);
                return true;
            }
            "\x1b[1;4C" | "\x1b[1;6C" => {
                let t = word_forward(&self.filter, self.filter_cursor) as i64;
                self.extend_to(t);
                return true;
            }
            _ => {}
        }

        // ---- edits — change filter, reset list selection + scroll ----
        if key == "\x7f" || key == "\x08" {
            if !self.delete_selection() {
                if self.filter_cursor == 0 {
                    return true;
                }
                self.filter.remove(self.filter_cursor - 1);
                self.filter_cursor -= 1;
            }
        } else if key == "\x1b[3~" {
            if !self.delete_selection() {
                if self.filter_cursor >= self.filter.len() {
                    return true;
                }
                self.filter.remove(self.filter_cursor);
            }
        } else if key == "\x1b\x7f" || key == "\x1b\x08" || key == "\x17" {
            if !self.delete_selection() {
                let start = word_back(&self.filter, self.filter_cursor);
                self.filter.drain(start..self.filter_cursor);
                self.filter_cursor = start;
            }
        } else if key == "\x15" {
            if !self.delete_selection() {
                self.filter.drain(0..self.filter_cursor);
                self.filter_cursor = 0;
            }
        } else if key == "\x0b" {
            if !self.delete_selection() {
                self.filter.truncate(self.filter_cursor);
            }
        } else if !key.is_empty() && key.chars().all(|c| c >= ' ' && c != '\u{7f}') {
            // Insert printable text. The original only inserted single-char
            // chunks; accepting any all-printable chunk makes fast typing and
            // paste robust (terminals coalesce keystrokes into one read).
            self.delete_selection();
            for c in key.chars() {
                self.filter.insert(self.filter_cursor, c);
                self.filter_cursor += 1;
            }
        } else {
            return false;
        }
        self.selected = 0;
        self.scroll = 0;
        true
    }

    fn handle_key(&mut self, key: &str, vis: &[Item]) {
        if self.handle_enter_or_exit(key, vis) {
            return;
        }
        if self.handle_navigation_key(key, vis) || self.handle_edit_key(key) {
            self.render();
        }
    }

    fn run(&mut self) {
        if !is_interactive() {
            eprintln!("palette requires an interactive terminal");
            process::exit(1);
        }
        let rm = match RawMode::enable() {
            Ok(rm) => rm,
            Err(e) => {
                eprintln!("tmux-palette: failed to enter raw mode: {}", e);
                process::exit(1);
            }
        };
        self.raw_mode = Some(rm);
        raw::install_signal_handlers();
        write_stdout(b"\x1b[?1000h\x1b[?1006h");
        self.render();

        let mut buf = [0u8; 4096];
        loop {
            if raw::should_exit() {
                self.exit_now();
            }
            match read_stdin(&mut buf) {
                Ok(0) => self.exit_now(),
                Ok(n) => {
                    let key = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let vis = self.visible();
                    if let Some((button, x, y, kind)) = parse_mouse_event(&key) {
                        self.handle_mouse(button, x, y, kind, &vis);
                    } else {
                        self.handle_key(&key, &vis);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    if raw::should_exit() {
                        self.exit_now();
                    }
                    if raw::take_winch() {
                        self.render();
                    }
                }
                Err(_) => self.exit_now(),
            }
        }
    }
}

/// Run the interactive palette to completion (exits the process when done).
pub fn run_palette(def: PaletteDef, loader: Option<PaletteLoader>, initial_name: &str) {
    let mut runner = Runner::new(def, loader, initial_name);
    runner.run();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sgr_mouse_press() {
        assert_eq!(parse_mouse_event("\x1b[<0;12;5M"), Some((0, 12, 5, 'M')));
        assert_eq!(parse_mouse_event("\x1b[<64;1;1m"), Some((64, 1, 1, 'm')));
        assert_eq!(parse_mouse_event("\x1b[A"), None);
    }

    #[test]
    fn nav_keys_map_to_deltas() {
        assert_eq!(nav_delta("\x1b[A"), Some(-1));
        assert_eq!(nav_delta("\x1b[B"), Some(1));
        assert_eq!(nav_delta("\x10"), Some(-1));
        assert_eq!(nav_delta("\x0e"), Some(1));
        assert_eq!(nav_delta("\x1b[5~"), Some(-10));
        assert_eq!(nav_delta("\x1b[6~"), Some(10));
        assert_eq!(nav_delta("x"), None);
    }

    #[test]
    fn word_motions() {
        let s: Vec<char> = "split horizontal".chars().collect();
        assert_eq!(word_back(&s, 16), 6);
        assert_eq!(word_back(&s, 5), 0);
        assert_eq!(word_forward(&s, 0), 5);
        assert_eq!(word_forward(&s, 6), 16);
    }

    #[test]
    fn drop_query_only_hides_until_search() {
        let mk = |title: &str, q: bool| Item {
            title: title.into(),
            query_only: q,
            ..Default::default()
        };
        let items = vec![mk("Split", false), mk("nvim", true), mk("Find Pane", false)];
        let resting: Vec<String> = drop_query_only(&items)
            .into_iter()
            .map(|i| i.title)
            .collect();
        assert_eq!(resting, vec!["Split".to_string(), "Find Pane".to_string()]);
    }

    #[test]
    fn footer_text_matches_count() {
        assert_eq!(build_footer_text(0, "No panes"), "No panes");
        assert_eq!(
            build_footer_text(1, "x"),
            "enter select   up/down move   1 command"
        );
        assert_eq!(
            build_footer_text(3, "x"),
            "enter select   up/down move   3 commands"
        );
    }
}
