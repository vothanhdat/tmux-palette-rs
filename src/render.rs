//! Row composition + ANSI styling — port of `src/render.ts`.

use crate::text::{char_width, clip, display_width, truncate};
use crate::types::{Colors, Item};

/// Cells a description needs before it is worth showing: the ` - ` separator
/// plus enough letters to say something. Below this the row drops the
/// description rather than render a lone ellipsis.
const MIN_DESC_W: i64 = 12;

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

/// The dim ` - <description>` tail, clipped to the cells the rest of the row
/// left over and dropped entirely when there are too few. Without the budget an
/// overlong description pushes the row past the popup width, and `truncate`'s cut
/// path then strips the ANSI off the *whole* row, not just the overflow.
fn description_fragment(item: &Item, colors: &Colors, row_bg: &str, budget: i64) -> (String, i64) {
    let Some(d) = item.description.as_deref().filter(|d| !d.is_empty()) else {
        return (String::new(), 0);
    };
    if budget < MIN_DESC_W {
        return (String::new(), 0);
    }
    let text = clip(d, budget - 3);
    let width = 3 + display_width(&text);
    (
        format!("{} - {}{}{}", colors.muted, text, colors.reset, row_bg),
        width,
    )
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
    render_item_styled(item, colors, active, body_width, None)
}

/// Like [`render_default_item`], but with an optional pre-built ANSI color for
/// the icon glyph that wins over the item's own `icon_color`/accent fallback.
/// Lets callers (e.g. inlined panes) tint the marker per the live theme.
pub fn render_item_styled(
    item: &Item,
    colors: &Colors,
    active: bool,
    body_width: i64,
    icon_override: Option<&str>,
) -> String {
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
    let icon_color = match icon_override {
        Some(c) => c.to_string(),
        None => item
            .icon_color
            .as_deref()
            .and_then(hex_to_fg)
            .unwrap_or_else(|| {
                if active {
                    active_hi.to_string()
                } else {
                    colors.accent.clone()
                }
            }),
    };
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
    let (sc_styled, sc_text) = shortcut_fragment(item, colors, active, row_bg);

    let icon_glyph_w = icon_glyph
        .as_deref()
        .map_or(1, |g| g.chars().next().map_or(1, char_width));
    let head_w = 1 + 1 + icon_glyph_w + 2 + display_width(&item.title) + chip_w;
    // The description gets what the fixed parts of the row leave, less the one
    // cell that always separates it from the shortcut column.
    let desc_budget = body_width - head_w - len(&sc_text) - 1;
    let (desc_styled, desc_w) = description_fragment(item, colors, row_bg, desc_budget);

    let left_styled = format!(
        "{} {}  {}{}{}",
        marker, icon, title_styled, chip_styled, desc_styled
    );
    let gap = (body_width - head_w - desc_w - len(&sc_text)).max(1);
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

/// Cells the gutter between the list and the preview occupies: space, rule, space.
const GUTTER_W: i64 = 3;
/// A list narrower than this loses the tree structure the panes are drawn with.
const MIN_LIST_W: i64 = 28;
/// A preview narrower than this shows too little of a line to be worth reading.
const MIN_PREVIEW_W: i64 = 24;
/// Below this the preview is all chrome and no content — see `PreviewCtx.height`.
const MIN_PREVIEW_H: i64 = 7;

/// Split the body between the list and a preview column, or `None` when the
/// popup is too small to carry both and the list should keep the whole width.
pub fn split_body(body_width: i64, body_height: i64) -> Option<(i64, i64)> {
    if body_height < MIN_PREVIEW_H {
        return None;
    }
    let list_w = (((body_width - GUTTER_W) * 45) / 100).max(MIN_LIST_W);
    let preview_w = body_width - GUTTER_W - list_w;
    (preview_w >= MIN_PREVIEW_W).then_some((list_w, preview_w))
}

/// The pre-rendered right-hand column, one entry per visible body row.
pub struct PreviewCol<'a> {
    pub lines: &'a [String],
    pub width: i64,
}

/// One body line: the list column on its row background, then — when a preview
/// is shown — a gutter and the preview column, both always on the panel
/// background so the selection bar stops at the list's edge.
fn body_line(
    content: &str,
    is_selected: bool,
    list_width: i64,
    pad_x: i64,
    colors: &Colors,
    preview: Option<(&str, i64)>,
) -> String {
    let row_bg = if is_selected {
        &colors.selected
    } else {
        &colors.panel
    };
    let left = format!(
        "{}{}{}",
        row_bg,
        spaces(pad_x),
        truncate(content, list_width)
    );
    match preview {
        None => format!("{}{}{}", left, spaces(pad_x), colors.reset),
        Some((line, width)) => format!(
            "{}{}{} {}│{}{} {}{}{}{}{}",
            left,
            colors.reset,
            colors.panel,
            colors.muted,
            colors.reset,
            colors.panel,
            // Fallback fg for a preview line long enough that `truncate` cut it,
            // dropping its own styling along with the overflow.
            colors.fg,
            truncate(line, width),
            colors.panel,
            spaces(pad_x),
            colors.reset
        ),
    }
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
    list_width: i64,
    pad_x: i64,
    colors: &Colors,
    start_y: i64,
    preview: Option<&PreviewCol>,
    render_row: F,
) -> ListBody {
    let mut lines = Vec::new();
    let mut row_actions = Vec::new();

    // The preview is indexed by position within the visible body, not by the
    // scrolled row index — it describes the selection, not the rows it sits by.
    let preview_at = |body_row: usize| {
        preview.map(|p| (p.lines.get(body_row).map_or("", |s| s.as_str()), p.width))
    };

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
        let content = render_row(row, is_selected);
        lines.push(body_line(
            &content,
            is_selected,
            list_width,
            pad_x,
            colors,
            preview_at(i - scroll),
        ));
    }
    while lines.len() < list_height {
        let body_row = lines.len();
        lines.push(body_line(
            "",
            false,
            list_width,
            pad_x,
            colors,
            preview_at(body_row),
        ));
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

    fn described(title: &str, description: &str) -> Item {
        Item {
            title: title.into(),
            description: Some(description.into()),
            action: Action::Shell(":".into()),
            ..Default::default()
        }
    }

    /// An overlong description used to push the row past the popup width, and
    /// `body_line`'s `truncate` then stripped the ANSI off the whole row. Clipping
    /// it here keeps the row exactly one body wide, so that cut never happens.
    #[test]
    fn a_long_description_is_clipped_before_it_can_tear_the_row() {
        let colors = Colors {
            muted: "\x1b[2m".into(),
            reset: "\x1b[0m".into(),
            ..Default::default()
        };
        let item = described("new-pane", "create a floating pane, wherever you like");
        let row = render_default_item(&item, &colors, false, 40);
        assert_eq!(display_width(&row), 40);
        assert!(row.contains('…'));
        assert!(row.contains("\x1b[2m"), "styling was stripped: {row:?}");
    }

    #[test]
    fn a_narrow_row_drops_the_description_rather_than_show_a_stub() {
        let item = described("new-pane", "create a floating pane");
        let row = render_default_item(&item, &plain_colors(), false, 16);
        assert!(!row.contains(" - "), "kept a useless stub: {row:?}");
        assert!(row.contains("new-pane"));
        assert_eq!(display_width(&row), 16);
    }

    #[test]
    fn a_description_that_fits_is_left_whole() {
        let item = described("kill-pane", "close a pane");
        let row = render_default_item(&item, &plain_colors(), false, 60);
        assert!(row.contains(" - close a pane"));
        assert_eq!(display_width(&row), 60);
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

    fn title_of(row: &Row, _: bool) -> String {
        match row {
            Row::Category { category } => category.clone(),
            Row::Item { item, .. } => item.title.clone(),
        }
    }

    #[test]
    fn list_body_tracks_only_item_rows() {
        let rows = build_rows(&sample_items(), true, false);
        let body = compose_list_body(&rows, 0, 3, 0, 20, 1, &plain_colors(), 10, None, title_of);
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

    #[test]
    fn split_body_gives_the_list_45_percent() {
        // Default popup: width 90, pad 3 -> body 84.
        assert_eq!(split_body(84, 20), Some((36, 45)));
        assert_eq!(36 + GUTTER_W + 45, 84);
    }

    #[test]
    fn split_body_declines_when_the_popup_is_too_small() {
        // Too narrow: the list floors at 28, leaving the preview under 24.
        assert_eq!(split_body(50, 20), None);
        // Too short: the panel would be all chrome and no pane content.
        assert_eq!(split_body(84, MIN_PREVIEW_H - 1), None);
        assert!(split_body(84, MIN_PREVIEW_H).is_some());
    }

    #[test]
    fn split_body_floors_the_list_before_conceding() {
        let (list, preview) = split_body(56, 20).unwrap();
        assert_eq!(list, MIN_LIST_W);
        assert_eq!(preview, 56 - GUTTER_W - MIN_LIST_W);
        assert!(preview >= MIN_PREVIEW_W);
    }

    /// Every body line must fill the popup exactly, or the panel background
    /// tears at the right edge.
    #[test]
    fn preview_lines_pad_the_body_to_full_width() {
        let rows = build_rows(&sample_items(), false, true);
        let preview = vec!["one".to_string(), "two".to_string()];
        let pad_x = 3;
        let (list_w, preview_w) = split_body(84, 20).unwrap();
        let body = compose_list_body(
            &rows,
            0,
            5,
            0,
            list_w,
            pad_x,
            &plain_colors(),
            10,
            Some(&PreviewCol {
                lines: &preview,
                width: preview_w,
            }),
            title_of,
        );
        assert_eq!(body.lines.len(), 5);
        for line in &body.lines {
            assert_eq!(display_width(line), pad_x + 84 + pad_x);
        }
        assert!(body.lines[0].contains("one"));
        assert!(body.lines[1].contains("two"));
        // Rows past the end of the preview still draw the gutter, so the column
        // keeps its edge all the way down.
        assert!(body.lines[4].contains('│'));
    }

    /// The selection bar stops at the list's edge — it must not bleed into the
    /// preview, which always sits on the panel background.
    #[test]
    fn selection_highlight_stops_at_the_preview() {
        let colors = Colors {
            selected: "SEL".into(),
            panel: "PANEL".into(),
            reset: "RESET".into(),
            ..Default::default()
        };
        let line = body_line("row", true, 10, 1, &colors, Some(("pv", 6)));
        let (left, right) = line.split_once('│').unwrap();
        assert!(left.contains("SEL"));
        assert!(!right.contains("SEL"));
        assert!(right.contains("pv"));
    }
}
