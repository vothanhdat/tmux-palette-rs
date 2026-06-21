//! The main `commands` palette — port of `src/palettes/commands.ts`.
//!
//! Generated from the original item list; icons are the same nerd-font glyphs.

use crate::types::{Action, Item, ItemsSource, PaletteDef};

pub fn commands() -> PaletteDef {
    PaletteDef {
        title: Some("Commands".to_string()),
        items: ItemsSource::Static(items()),
        ..Default::default()
    }
}

fn items() -> Vec<Item> {
    vec![
        Item::cmd("󰍉", "Panes", "Find Pane", Action::Palette("find-pane".to_string())),
        Item::cmd("", "Panes", "Split Horizontal", Action::Tmux("split-window -h -c '#{pane_current_path}'".to_string())).desc("side by side"),
        Item::cmd("", "Panes", "Split Vertical", Action::Tmux("split-window -v -c '#{pane_current_path}'".to_string())).desc("stacked"),
        Item::cmd("󰅖", "Panes", "Close Pane", Action::Tmux("kill-pane".to_string())),
        Item::cmd("󰒉", "Panes", "Close Other Panes", Action::Tmux("confirm-before -p 'kill all other panes? (y/n)' 'kill-pane -a'".to_string())),
        Item::cmd("󰁔", "Panes", "Next Pane", Action::Tmux("select-pane -t +1".to_string())),
        Item::cmd("󰁍", "Panes", "Previous Pane", Action::Tmux("select-pane -t -1".to_string())),
        Item::cmd("󰎠", "Panes", "Display Pane Numbers", Action::Tmux("display-panes".to_string())),
        Item::cmd("󰓡", "Panes", "Cycle Pane Layout", Action::Tmux("next-layout".to_string())),
        Item::cmd("󰁝", "Panes", "Swap Pane Up", Action::Tmux("swap-pane -U".to_string())),
        Item::cmd("󰁅", "Panes", "Swap Pane Down", Action::Tmux("swap-pane -D".to_string())),
        Item::cmd("󰍉", "Panes", "Zoom / Unzoom", Action::Tmux("resize-pane -Z".to_string())),
        Item::cmd("󰆏", "Panes", "Enter Copy Mode", Action::Tmux("copy-mode".to_string())).desc("scrollback / select"),
        Item::cmd("󰏫", "Panes", "Rename Pane", Action::Tmux("command-prompt -I '#{pane_title}' 'select-pane -T \"%1\"'".to_string())),
        Item::cmd("󰁁", "Panes", "Move Pane to...", Action::Palette("move-pane".to_string())),
        Item::cmd("󰘖", "Panes", "Break to New Window", Action::Tmux("break-pane".to_string())),
        Item::cmd("󰝰", "Windows", "New Window", Action::Tmux("new-window -c '#{pane_current_path}'".to_string())),
        Item::cmd("󰁔", "Windows", "Next Window", Action::Tmux("next-window".to_string())),
        Item::cmd("󰁍", "Windows", "Previous Window", Action::Tmux("previous-window".to_string())),
        Item::cmd("󰋚", "Windows", "Last Window", Action::Tmux("last-window".to_string())),
        Item::cmd("󰏫", "Windows", "Rename Window", Action::Tmux("command-prompt -I '#W' 'rename-window -- \"%%\"'".to_string())),
        Item::cmd("󰅖", "Windows", "Close Window", Action::Tmux("confirm-before -p 'kill window? (y/n)' kill-window".to_string())),
        Item::cmd("󱂬", "Sessions", "Choose Session", Action::Tmux("choose-tree -Zs".to_string())),
        Item::cmd("󰐕", "Sessions", "New Session", Action::Tmux("command-prompt -p 'New session name:' 'new-session -d -s \"%1\" ; switch-client -t \"%1\"'".to_string())),
        Item::cmd("󰏫", "Sessions", "Rename Session", Action::Tmux("command-prompt -I '#S' 'rename-session -- \"%%\"'".to_string())),
        Item::cmd("󰁔", "Sessions", "Next Session", Action::Tmux("switch-client -n".to_string())),
        Item::cmd("󰁍", "Sessions", "Previous Session", Action::Tmux("switch-client -p".to_string())),
        Item::cmd("󰍃", "Sessions", "Detach", Action::Tmux("detach-client".to_string())),
        Item::cmd("󰆴", "Sessions", "Kill Session", Action::Tmux("confirm-before -p 'kill session #S? (y/n)' kill-session".to_string())),
        Item::cmd("󰑓", "System", "Reload Config", Action::Tmux("source-file ~/.tmux.conf ; display-message 'Config reloaded'".to_string())),
        Item::cmd("", "Appearance", "Switch Theme...", Action::Palette("themes".to_string())).desc("browse + live-preview bundled themes"),
    ]
}
