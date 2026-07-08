//! `command-prompt` palette — a fuzzy replacement for tmux's `prefix + :`
//! command prompt.
//!
//! The command list is pulled live from `tmux list-commands`, so *every* tmux
//! command is searchable (name, alias, and argument syntax). Selecting one runs
//! it immediately when it needs no arguments (only optional flags), or completes
//! `<name> ` into the input when it has required arguments so you can finish the
//! line and run it. A synthetic "Run: <what you typed>" row is always offered
//! too, so any typed command dispatches as `tmux <line>` after the popup closes
//! (the same after-popup trick that lets interactive prompts receive stdin).

use std::rc::Rc;

use crate::fuzzy::default_filter;
use crate::tmux::tmux;
use crate::types::{Action, Item, ItemsSource, PaletteDef};

/// Marker for the synthetic row that runs exactly what the user typed.
const RUN_ICON: &str = "▶";
/// Icon for a tmux command row.
const CMD_ICON: &str = "";

/// True when `usage` lists a required positional argument — anything left once
/// the `[optional]` groups (which nest) are stripped out. Such commands are
/// completed into the input rather than run bare.
fn has_required_args(usage: &str) -> bool {
    let mut depth = 0i32;
    let mut bare = String::new();
    for c in usage.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => bare.push(c),
            _ => {}
        }
    }
    !bare.trim().is_empty()
}

/// Parse one `tmux list-commands` line: `name [(alias)] <usage...>`.
fn parse_command_line(line: &str) -> Option<Item> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let name_end = line.find(char::is_whitespace).unwrap_or(line.len());
    let name = &line[..name_end];
    let mut rest = line[name_end..].trim_start();

    let mut alias = None;
    if let Some(stripped) = rest.strip_prefix('(') {
        if let Some(close) = stripped.find(')') {
            alias = Some(stripped[..close].to_string());
            rest = stripped[close + 1..].trim_start();
        }
    }
    let usage = rest.trim();

    let action = if has_required_args(usage) {
        // Complete the command into the input, ready for arguments.
        Action::Fill(format!("{} ", name))
    } else {
        // No required args — safe to run as-is.
        Action::Tmux(name.to_string())
    };

    Some(Item {
        icon: Some(CMD_ICON.to_string()),
        title: name.to_string(),
        description: if usage.is_empty() {
            None
        } else {
            Some(usage.to_string())
        },
        aliases: alias.map(|a| vec![a]),
        action,
        ..Default::default()
    })
}

/// Every tmux command, straight from `tmux list-commands`.
fn build_items() -> Vec<Item> {
    tmux(&["list-commands"])
        .lines()
        .filter_map(parse_command_line)
        .collect()
}

/// The row that runs exactly what the user typed — the core `prefix + :`
/// behavior. `query` is the already-trimmed search text.
fn run_typed_item(query: &str) -> Item {
    Item {
        icon: Some(RUN_ICON.to_string()),
        title: format!("Run: {}", query),
        description: Some("run as a tmux command".to_string()),
        action: Action::Tmux(query.to_string()),
        ..Default::default()
    }
}

/// Filter: fuzzy-rank the commands and always offer a "Run: <query>" row on top,
/// so any typed command is runnable. A command whose name exactly equals the
/// query is dropped to avoid a duplicate of the run row.
fn filter_commands(items: &[Item], query: &str) -> Vec<Item> {
    let matched: Vec<Item> = default_filter(items, query)
        .into_iter()
        .filter(|i| i.title != query)
        .collect();
    let mut out = Vec::with_capacity(matched.len() + 1);
    out.push(run_typed_item(query));
    out.extend(matched);
    out
}

pub fn command_prompt() -> PaletteDef {
    PaletteDef {
        title: Some("Run Command".to_string()),
        grouped: Some(false),
        empty_text: Some("Type a tmux command".to_string()),
        items: ItemsSource::Dynamic(Rc::new(build_items)),
        filter: Some(Rc::new(filter_commands)),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_args_detected_ignoring_optional_groups() {
        // Only optional flags/args -> no required args.
        assert!(!has_required_args(
            "[-dErx] [-c working-directory] [-t target-session]"
        ));
        assert!(!has_required_args(""));
        // Bare positional (possibly after optional groups) -> required.
        assert!(has_required_args("[-b] [-p prompt] command"));
        assert!(has_required_args("[-t target-window] new-name"));
        // Nested optional groups strip fully.
        assert!(!has_required_args(
            "[-nr] [-T key-table] [command [arguments]]"
        ));
    }

    #[test]
    fn parses_command_with_alias_and_required_arg() {
        let item = parse_command_line(
            "confirm-before (confirm) [-b] [-p prompt] [-t target-client] command",
        )
        .unwrap();
        assert_eq!(item.title, "confirm-before");
        assert_eq!(item.aliases.as_deref(), Some(&["confirm".to_string()][..]));
        // Needs an argument -> completes into the input.
        match &item.action {
            Action::Fill(t) => assert_eq!(t, "confirm-before "),
            _ => panic!("expected a Fill action"),
        }
        assert!(item.description.as_deref().unwrap().contains("command"));
    }

    #[test]
    fn parses_no_arg_command_as_runnable() {
        let item = parse_command_line(
            "detach-client (detach) [-aP] [-E shell-command] [-t target-client]",
        )
        .unwrap();
        assert_eq!(item.title, "detach-client");
        match &item.action {
            Action::Tmux(c) => assert_eq!(c, "detach-client"),
            _ => panic!("expected a Tmux action"),
        }
    }

    #[test]
    fn parses_command_without_alias() {
        let item =
            parse_command_line("choose-tree [-GNrswZ] [-F format] [-t target-pane]").unwrap();
        assert_eq!(item.title, "choose-tree");
        assert_eq!(item.aliases, None);
        assert!(matches!(item.action, Action::Tmux(_)));
    }

    #[test]
    fn filter_prepends_run_row_for_arbitrary_input() {
        let items = vec![parse_command_line("kill-pane (killp) [-a] [-t target-pane]").unwrap()];
        let vis = filter_commands(&items, "new-session -s work");
        assert_eq!(vis[0].title, "Run: new-session -s work");
        match &vis[0].action {
            Action::Tmux(c) => assert_eq!(c, "new-session -s work"),
            _ => panic!("run row must be a tmux action"),
        }
    }

    #[test]
    fn exact_query_is_not_duplicated_below_the_run_row() {
        let items = vec![parse_command_line("kill-pane (killp) [-a] [-t target-pane]").unwrap()];
        let vis = filter_commands(&items, "kill-pane");
        assert_eq!(vis[0].title, "Run: kill-pane");
        assert!(!vis[1..].iter().any(|i| i.title == "kill-pane"));
    }
}
