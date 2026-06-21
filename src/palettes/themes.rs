//! `themes` palette — theme switcher with live preview — port of
//! `src/palettes/themes.ts`.

use std::any::Any;
use std::fs;
use std::rc::Rc;

use crate::theme::list_themes;
use crate::types::{Action, Item, ItemsSource, PaletteDef, Theme};
use crate::user_config::config_dir;

const CUSTOM_THEME_DOCS: &str = "https://github.com/vothanhdat/tmux-palette-rs#themes";

fn save_theme(slug: &str) {
    let path = format!("{}/theme.json", config_dir());
    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = fs::create_dir_all(parent);
    }
    let content = serde_json::to_string_pretty(&serde_json::json!({ "name": slug }))
        .unwrap_or_else(|_| format!("{{\"name\":\"{}\"}}", slug));
    let _ = fs::write(&path, format!("{}\n", content));
}

fn build_items() -> Vec<Item> {
    let mut items: Vec<Item> = list_themes()
        .into_iter()
        .map(|t| {
            let slug = t.slug.clone();
            Item {
                icon: Some("\u{25cf}".to_string()),
                icon_color: Some(t.theme.accent.clone()),
                title: t.name,
                description: if t.source == "user" {
                    Some("custom".to_string())
                } else {
                    None
                },
                aliases: Some(vec![t.slug]),
                data: Some(Rc::new(t.theme) as Rc<dyn Any>),
                action: Action::Apply(Rc::new(move |_ctx| save_theme(&slug))),
                ..Default::default()
            }
        })
        .collect();

    items.push(Item {
        icon: Some("+".to_string()),
        title: "Add custom theme...".to_string(),
        description: Some("Open setup instructions".to_string()),
        aliases: Some(vec![
            "custom".to_string(),
            "theme".to_string(),
            "docs".to_string(),
        ]),
        action: Action::Shell(format!("open '{0}' || xdg-open '{0}'", CUSTOM_THEME_DOCS)),
        ..Default::default()
    });

    items
}

pub fn themes() -> PaletteDef {
    PaletteDef {
        title: Some("Themes".to_string()),
        grouped: Some(false),
        items: ItemsSource::Dynamic(Rc::new(build_items)),
        empty_text: Some("No themes found".to_string()),
        on_select: Some(Rc::new(|item: Option<&Item>| -> Option<Theme> {
            item.and_then(|i| i.data.as_ref())
                .and_then(|d| d.downcast_ref::<Theme>())
                .cloned()
        })),
        ..Default::default()
    }
}
