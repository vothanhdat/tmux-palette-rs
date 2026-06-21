//! Row composition + ANSI styling — port of `src/render.ts`.

use crate::text::{char_width, display_width, truncate};
use crate::types::{Colors, Item};

fn spaces(n: i64) -> String {
    " ".repeat(n.max(0) as usize)
}

/// Char count used where the TS code reads `String.length` (ASCII-dominant).
fn len(s: &str) -> i64 {
    s.chars().count() as i64
}

fn hex_to_fg(hex: &str) -> Option<String> {
    let s = hex.strip_prefix('#').unwrap_or(hex);
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(format!("\x1b[38;2;{};{};{}m", r, g, b))
}

#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Row {
    Category { category: String },
    Item { item: Item, item_index: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowAction {
    pub y: i64,
    pub item_index: usize,
}

pub fn is_selectable(item: Option<&Item>) -> bool {
    match item {
        Some(it) => it.selectable != Some(false),
        None => false,
    }
}

pub fn step(vis: &[Item], from: usize, dir: i64) -> usize {
    if vis.is_empty() {
        return 0;
    }
    let n = vis.len() as i64;
    let mut i = from as i64;
    for _ in 0..vis.len() {
        i = (i + dir + n) % n;
        if is_selectable(Some(&vis[i as usize])) {
            return i as usize;
        }
    }
    from
}

pub fn first_selectable(vis: &[Item]) -> i64 {
    for (i, item) in vis.iter().enumerate() {
        if is_selectable(Some(item)) {
            return i as i64;
        }
    }
    -1
}

pub fn build_rows(vis: &[Item], grouped: bool, filtered: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut last_cat = String::new();
    for (i, item) in vis.iter().enumerate() {
        if grouped && !filtered {
            if let Some(cat) = &item.category {
                if !cat.is_empty() && *cat != last_cat {
                    rows.push(Row::Category {
                        category: cat.clone(),
                    });
                    last_cat = cat.clone();
                }
            }
        }
        rows.push(Row::Item {
            item: item.clone(),
            item_index: i,
        });
    }
    rows
}

pub fn render_category(category: &str, colors: &Colors, row_bg: &str) -> String {
    format!(
        "{}{}{}{}{}",
        colors.accent, colors.bold, category, colors.reset, row_bg
    )
}

fn alias_chip(item: &Item, colors: &Colors, row_bg: &str) -> (String, i64) {
    match item.aliases.as_ref().and_then(|a| a.first()) {
        Some(alias) if !alias.is_empty() => (
            format!(
                "  {}{} {} {}{}",
                colors.bg, colors.fg, alias, colors.reset, row_bg
            ),
            2 + 1 + len(alias) + 1,
        ),
        _ => (String::new(), 0),
    }
}

fn description_fragment(item: &Item, colors: &Colors, row_bg: &str) -> (String, i64) {
    match &item.description {
        Some(d) if !d.is_empty() => (
            format!("{} - {}{}{}", colors.muted, d, colors.reset, row_bg),
            3 + len(d),
        ),
        _ => (String::new(), 0),
    }
}

fn shortcut_fragment(item: &Item, colors: &Colors, active: bool, row_bg: &str) -> (String, String) {
    let text = item.shortcut.clone().unwrap_or_default();
    if text.is_empty() {
        return (String::new(), text);
    }
    let color = if active {
        if colors.selected_fg.is_empty() {
            &colors.accent
        } else {
            &colors.selected_fg
        }
    } else {
        &colors.muted
    };
    (format!("{}{}{}{}", color, text, colors.reset, row_bg), text)
}

pub fn render_default_item(item: &Item, colors: &Colors, active: bool, body_width: i64) -> String {
    let row_bg = if active {
        &colors.selected
    } else {
        &colors.panel
    };
    let active_hi: &str = if colors.selected_fg.is_empty() {
        &colors.accent
    } else {
        &colors.selected_fg
    };
    let marker = if active {
        format!("{}▌{}{}", active_hi, colors.reset, row_bg)
    } else {
        " ".to_string()
    };
    let icon_glyph = item.icon.clone().filter(|i| !i.is_empty());
    let icon_color = item
        .icon_color
        .as_deref()
        .and_then(hex_to_fg)
        .unwrap_or_else(|| {
            if active {
                active_hi.to_string()
            } else {
                colors.accent.clone()
            }
        });
    let icon = match &icon_glyph {
        Some(g) => format!("{}{}{}{}", icon_color, g, colors.reset, row_bg),
        None => " ".to_string(),
    };
    let title_style = if active {
        format!(
            "{}{}",
            colors.bold,
            if colors.selected_fg.is_empty() {
                &colors.fg
            } else {
                &colors.selected_fg
            }
        )
    } else {
        colors.muted.clone()
    };
    let title_styled = format!("{}{}{}{}", title_style, item.title, colors.reset, row_bg);

    let (chip_styled, chip_w) = alias_chip(item, colors, row_bg);
    let (desc_styled, desc_w) = description_fragment(item, colors, row_bg);
    let (sc_styled, sc_text) = shortcut_fragment(item, colors, active, row_bg);

    let left_styled = format!(
        "{} {}  {}{}{}",
        marker, icon, title_styled, chip_styled, desc_styled
    );
    let icon_glyph_w = icon_glyph
        .as_deref()
        .map_or(1, |g| g.chars().next().map_or(1, char_width));
    let left_plain_w = 1 + 1 + icon_glyph_w + 2 + display_width(&item.title) + chip_w + desc_w;

    let gap = (body_width - left_plain_w - len(&sc_text)).max(1);
    format!("{}{}{}", left_styled, spaces(gap), sc_styled)
}

pub struct HeaderResult {
    pub line: String,
    pub esc_x1: i64,
    pub esc_x2: i64,
}

pub fn compose_header(
    title: &str,
    width: i64,
    pad_x: i64,
    body_width: i64,
    colors: &Colors,
) -> HeaderResult {
    let header_r = "esc";
    let header_rw = display_width(header_r);
    let header_gap = (body_width - display_width(title) - header_rw).max(0);
    let title_fg = if colors.title_fg.is_empty() {
        &colors.fg
    } else {
        &colors.title_fg
    };
    let line = format!(
        "{}{}{}{}{}{}{}{}{}esc{}{}{}",
        colors.panel,
        spaces(pad_x),
        colors.bold,
        title_fg,
        title,
        colors.reset,
        colors.panel,
        spaces(header_gap),
        colors.muted,
        colors.panel,
        spaces(pad_x),
        colors.reset
    );
    HeaderResult {
        line,
        esc_x1: (width - pad_x - header_rw).max(1),
        esc_x2: width - pad_x + 1,
    }
}

pub fn compose_search(
    filter: &str,
    pad_x: i64,
    body_width: i64,
    colors: &Colors,
    sel_start: Option<usize>,
    sel_end: Option<usize>,
) -> String {
    let pad = spaces(pad_x);
    if filter.is_empty() {
        return format!(
            "{}{}{}▌{} {}{}{}{}",
            colors.panel,
            pad,
            colors.accent,
            colors.muted,
            truncate("Search", body_width - 2),
            colors.panel,
            pad,
            colors.reset
        );
    }
    let text = truncate(filter, body_width - 2);
    let has_selection = matches!((sel_start, sel_end), (Some(a), Some(b)) if a < b);
    if !has_selection {
        return format!(
            "{}{}{}▌{} {}{}{}{}",
            colors.panel, pad, colors.accent, colors.fg, text, colors.panel, pad, colors.reset
        );
    }
    let chars: Vec<char> = text.chars().collect();
    let total = chars.len();
    let a = sel_start.unwrap().min(total);
    let b = sel_end.unwrap().clamp(a, total);
    let before: String = chars[..a].iter().collect();
    let inside: String = chars[a..b].iter().collect();
    let after: String = chars[b..].iter().collect();
    format!(
        "{}{}{}▌{} {}{}{}{}{}{}{}{}",
        colors.panel,
        pad,
        colors.accent,
        colors.fg,
        before,
        colors.selected,
        inside,
        colors.panel,
        colors.fg,
        after,
        pad,
        colors.reset
    )
}

fn render_list_row<F: Fn(&Row, bool) -> String>(
    row: &Row,
    is_selected: bool,
    body_width: i64,
    pad_x: i64,
    colors: &Colors,
    render_row: &F,
) -> String {
    let row_bg = if is_selected {
        &colors.selected
    } else {
        &colors.panel
    };
    let content = render_row(row, is_selected);
    format!(
        "{}{}{}{}{}",
        row_bg,
        spaces(pad_x),
        truncate(&content, body_width),
        spaces(pad_x),
        colors.reset
    )
}

pub struct ListBody {
    pub lines: Vec<String>,
    pub row_actions: Vec<RowAction>,
}

#[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
pub fn compose_list_body<F: Fn(&Row, bool) -> String>(
    rows: &[Row],
    scroll: usize,
    list_height: usize,
    selected: usize,
    body_width: i64,
    pad_x: i64,
    colors: &Colors,
    start_y: i64,
    render_row: F,
) -> ListBody {
    let mut lines = Vec::new();
    let mut row_actions = Vec::new();

    let end = rows.len().min(scroll + list_height);
    for i in scroll..end {
        let row = &rows[i];
        let is_selected = matches!(row, Row::Item { item_index, .. } if *item_index == selected);
        if let Row::Item { item_index, .. } = row {
            row_actions.push(RowAction {
                y: start_y + (i as i64 - scroll as i64),
                item_index: *item_index,
            });
        }
        lines.push(render_list_row(
            row,
            is_selected,
            body_width,
            pad_x,
            colors,
            &render_row,
        ));
    }
    let blank = format!(
        "{}{}{}",
        colors.panel,
        spaces(body_width + pad_x * 2),
        colors.reset
    );
    while lines.len() < list_height {
        lines.push(blank.clone());
    }
    ListBody { lines, row_actions }
}

pub fn compose_footer(footer_text: &str, pad_x: i64, body_width: i64, colors: &Colors) -> String {
    format!(
        "{}{}{}{}{}{}{}",
        colors.panel,
        spaces(pad_x),
        colors.muted,
        truncate(footer_text, body_width),
        colors.panel,
        spaces(pad_x),
        colors.reset
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Action;

    fn sample_items() -> Vec<Item> {
        vec![
            Item {
                title: "Find Pane".into(),
                category: Some("Panes".into()),
                action: Action::Shell(":".into()),
                ..Default::default()
            },
            Item {
                title: "Section".into(),
                category: Some("Panes".into()),
                selectable: Some(false),
                action: Action::Shell(":".into()),
                ..Default::default()
            },
            Item {
                title: "New Window".into(),
                category: Some("Windows".into()),
                action: Action::Shell(":".into()),
                ..Default::default()
            },
        ]
    }

    fn plain_colors() -> Colors {
        Colors::default()
    }

    #[test]
    fn selectable_unless_disabled() {
        let items = sample_items();
        assert!(is_selectable(Some(&items[0])));
        assert!(!is_selectable(Some(&items[1])));
    }

    #[test]
    fn finds_and_steps_over_non_selectable() {
        let items = sample_items();
        assert_eq!(first_selectable(&items), 0);
        assert_eq!(step(&items, 0, 1), 2);
        assert_eq!(step(&items, 2, -1), 0);
    }

    #[test]
    fn build_rows_adds_categories_when_unfiltered() {
        let rows = build_rows(&sample_items(), true, false);
        let got: Vec<String> = rows
            .iter()
            .map(|r| match r {
                Row::Category { category } => category.clone(),
                Row::Item { item, .. } => item.title.clone(),
            })
            .collect();
        assert_eq!(
            got,
            vec!["Panes", "Find Pane", "Section", "Windows", "New Window"]
        );
    }

    #[test]
    fn build_rows_omits_categories_while_filtering() {
        let rows = build_rows(&sample_items(), true, true);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| matches!(r, Row::Item { .. })));
    }

    fn header_base() -> Colors {
        Colors {
            fg: "FG".into(),
            muted: "MUT".into(),
            accent: "ACC".into(),
            ..Default::default()
        }
    }

    #[test]
    fn header_title_uses_fg_when_title_fg_unset() {
        let r = compose_header("Commands", 40, 1, 38, &header_base());
        assert!(r.line.contains("FGCommands"));
    }

    #[test]
    fn header_title_uses_title_fg_when_set() {
        let mut c = header_base();
        c.title_fg = "MAG".into();
        let r = compose_header("Commands", 40, 1, 38, &c);
        assert!(r.line.contains("MAGCommands"));
        assert!(!r.line.contains("FGCommands"));
    }

    fn item_colors() -> Colors {
        Colors {
            fg: "FG".into(),
            muted: "MUT".into(),
            accent: "ACC".into(),
            ..Default::default()
        }
    }

    fn split_item() -> Item {
        Item {
            title: "Split".into(),
            icon: Some("I".into()),
            shortcut: Some("C-s".into()),
            action: Action::Shell(":".into()),
            ..Default::default()
        }
    }

    #[test]
    fn active_row_uses_accent_and_fg_without_selected_fg() {
        let out = render_default_item(&split_item(), &item_colors(), true, 40);
        assert!(out.contains("ACC▌"));
        assert!(out.contains("ACC"));
        assert!(out.contains("FGSplit"));
        assert!(!out.contains("SEL"));
    }

    #[test]
    fn active_row_uses_selected_fg_when_set() {
        let mut c = item_colors();
        c.selected_fg = "SEL".into();
        let out = render_default_item(&split_item(), &c, true, 40);
        assert!(out.contains("SEL▌"));
        assert!(out.contains("SELSplit"));
        assert!(out.matches("SEL").count() >= 3);
    }

    #[test]
    fn inactive_icon_stays_accent() {
        let mut c = item_colors();
        c.selected_fg = "SEL".into();
        let out = render_default_item(&split_item(), &c, false, 40);
        assert!(out.contains("ACC"));
        assert!(out.contains("MUTSplit"));
    }

    #[test]
    fn list_body_tracks_only_item_rows() {
        let rows = build_rows(&sample_items(), true, false);
        let body = compose_list_body(
            &rows,
            0,
            3,
            0,
            20,
            1,
            &plain_colors(),
            10,
            |row, _| match row {
                Row::Category { category } => category.clone(),
                Row::Item { item, .. } => item.title.clone(),
            },
        );
        assert_eq!(body.lines.len(), 3);
        assert_eq!(
            body.row_actions,
            vec![
                RowAction {
                    y: 11,
                    item_index: 0
                },
                RowAction {
                    y: 12,
                    item_index: 1
                },
            ]
        );
    }
}
