//! Core data types — port of `src/types.ts`.
//!
//! Closures from the TS version (`run`/`apply` actions and the dynamic palette
//! callbacks) become `Rc<dyn Fn…>` so values stay cheaply cloneable; the
//! TS `unknown` payload (`Item.data`) becomes `Rc<dyn Any>`.

use std::any::Any;
use std::rc::Rc;

/// Open a command in a centered tmux popup.
#[derive(Clone, Debug, Default)]
pub struct PopupAction {
    pub popup: String,
    pub width: Option<String>,
    pub height: Option<String>,
    pub pad_x: Option<i64>,
    pub pad_y: Option<i64>,
    pub border: Option<String>,
}

/// What happens when an item is activated.
#[derive(Clone)]
pub enum Action {
    /// Runs `tmux <cmd>` after the popup closes (so interactive prompts get stdin).
    Tmux(String),
    /// Runs a shell command after the popup closes.
    Shell(String),
    /// Chains into another palette (in-process navigation).
    Palette(String),
    /// Opens `popup` in a centered tmux popup.
    Popup(PopupAction),
    /// Runs in-process, then exits.
    Run(RunFn),
    /// Runs in-process WITHOUT closing the popup, then navigates back.
    Apply(RunFn),
}

pub type RunFn = Rc<dyn Fn(&ActionContext)>;

pub struct ActionContext {
    pub cmd_file: Option<String>,
}

#[derive(Clone)]
pub struct Item {
    pub icon: Option<String>,
    /// Optional hex color (e.g. `#22cc22`) applied to the icon glyph.
    pub icon_color: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub shortcut: Option<String>,
    pub category: Option<String>,
    pub aliases: Option<Vec<String>>,
    pub action: Action,
    /// Arbitrary payload for custom `render_item` implementations.
    pub data: Option<Rc<dyn Any>>,
    /// When `Some(false)`, the cursor skips this item (visual-only rows).
    pub selectable: Option<bool>,
}

impl Default for Item {
    fn default() -> Self {
        Item {
            icon: None,
            icon_color: None,
            title: String::new(),
            description: None,
            shortcut: None,
            category: None,
            aliases: None,
            action: Action::Shell(":".to_string()),
            data: None,
            selectable: None,
        }
    }
}

impl Item {
    /// Convenience constructor for the built-in command items.
    pub fn cmd(icon: &str, category: &str, title: &str, action: Action) -> Self {
        Item {
            icon: Some(icon.to_string()),
            category: Some(category.to_string()),
            title: title.to_string(),
            action,
            ..Default::default()
        }
    }

    pub fn desc(mut self, description: &str) -> Self {
        self.description = Some(description.to_string());
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    pub bg: String,
    pub panel: String,
    pub selected: String,
    pub fg: String,
    pub muted: String,
    pub accent: String,
    /// Foreground for the highlighted row (icon/title/marker/shortcut).
    pub selected_fg: Option<String>,
    /// Foreground for the popup title in the header.
    pub title_fg: Option<String>,
}

/// Pre-built ANSI escape sequences derived from a [`Theme`].
#[derive(Clone, Default)]
pub struct Colors {
    pub bg: String,
    pub panel: String,
    pub selected: String,
    pub fg: String,
    pub muted: String,
    pub accent: String,
    /// Highlight fg for the active row, or `""` to fall back to fg/accent.
    pub selected_fg: String,
    /// Fg for the header title, or `""` to fall back to fg.
    pub title_fg: String,
    pub reset: String,
    pub bold: String,
}

pub struct RenderItemCtx<'a> {
    pub colors: &'a Colors,
    pub active: bool,
    /// Body width available for the row (popup width minus horizontal padding).
    pub width: i64,
}

/// A palette's declared theme: a bundled/user slug, or a full theme literal.
#[derive(Clone)]
pub enum ThemeRef {
    Name(String),
    Full(Theme),
}

/// Items for a palette: either fixed, or produced by a callback at load time.
#[derive(Clone)]
pub enum ItemsSource {
    Static(Vec<Item>),
    Dynamic(Rc<dyn Fn() -> Vec<Item>>),
}

impl ItemsSource {
    pub fn resolve(&self) -> Vec<Item> {
        match self {
            ItemsSource::Static(v) => v.clone(),
            ItemsSource::Dynamic(f) => f(),
        }
    }
}

pub type RenderItemFn = Rc<dyn Fn(&Item, &RenderItemCtx) -> String>;
pub type FilterFn = Rc<dyn Fn(&[Item], &str) -> Vec<Item>>;
pub type OnSelectFn = Rc<dyn Fn(Option<&Item>) -> Option<Theme>>;
pub type InitialSelectedFn = Rc<dyn Fn(&[Item]) -> i64>;

#[derive(Clone)]
pub struct PaletteDef {
    pub title: Option<String>,
    pub items: ItemsSource,
    pub theme: Option<ThemeRef>,
    pub grouped: Option<bool>,
    pub empty_text: Option<String>,
    /// Custom row renderer; returns the row's ANSI-styled content.
    pub render_item: Option<RenderItemFn>,
    /// Custom filter (e.g. tree palettes that keep ancestors visible).
    pub filter: Option<FilterFn>,
    /// Live-preview hook: return a theme to swap colors before the next frame.
    pub on_select: Option<OnSelectFn>,
    /// Picks the initial highlighted item (returns an index, or -1).
    pub initial_selected: Option<InitialSelectedFn>,
}

impl Default for PaletteDef {
    fn default() -> Self {
        PaletteDef {
            title: None,
            items: ItemsSource::Static(Vec::new()),
            theme: None,
            grouped: None,
            empty_text: None,
            render_item: None,
            filter: None,
            on_select: None,
            initial_selected: None,
        }
    }
}

impl PaletteDef {
    pub fn resolve_items(&self) -> Vec<Item> {
        self.items.resolve()
    }
}
