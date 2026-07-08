//! `command-prompt` palette — a fuzzy replacement for tmux's `prefix + :`
//! command prompt.
//!
//! The command list is pulled live from `tmux list-commands`, so *every* tmux
//! command is searchable (name, alias, and argument syntax). Selecting a command
//! runs it immediately when it needs no arguments, or completes `<name> ` into
//! the input when it takes arguments. Tab cycles through the matches (see the
//! palette loop), and a synthetic "Run: <what you typed>" row always dispatches
//! the typed line via the after-popup trick that keeps interactive prompts fed.
//!
//! When the filter narrows to exactly one command, its arguments are expanded
//! below it — one param per line, tagged optional/required, with a short
//! description drawn from a bundled glossary of tmux's (very consistent)
//! placeholder names and common flags.

use std::rc::Rc;

use crate::fuzzy::default_filter;
use crate::render::render_default_item;
use crate::text::display_width;
use crate::tmux::tmux;
use crate::types::{Action, Colors, Item, ItemsSource, PaletteDef, RenderItemCtx};

/// Marker for the synthetic row that runs exactly what the user typed.
const RUN_ICON: &str = "▶";
/// Icon for a tmux command row.
const CMD_ICON: &str = "";

// ---- glossary -----------------------------------------------------------------

/// Short description for an argument placeholder (the token tmux prints, e.g.
/// `target-window`). tmux names these consistently across all commands, so this
/// covers essentially every argument. Unknown placeholders return `None`.
fn arg_desc(name: &str) -> Option<&'static str> {
    Some(match name {
        "target-pane" => "pane to target (default: current)",
        "target-window" => "window to target (default: current)",
        "target-session" => "session to target (default: current)",
        "target-client" => "client to target (default: attached)",
        "src-pane" => "source pane",
        "dst-pane" => "destination pane",
        "src-window" => "source window",
        "dst-window" => "destination window",
        "pane" => "pane",
        "format" => "output format string (#{...})",
        "filter" => "only keep items matching this format",
        "shell-command" => "shell command to run",
        "command" => "tmux command to run",
        "arguments" => "arguments for the command",
        "template" => "command template; %-fields are substituted",
        "buffer-name" | "new-buffer-name" => "paste-buffer name",
        "environment" => "environment entry (NAME=value)",
        "start-directory" | "working-directory" => "directory to start in",
        "key" => "key",
        "key-table" => "key table (root, prefix, copy-mode, …)",
        "key-format" => "key display format",
        "height" => "height in lines or N%",
        "width" => "width in columns or N%",
        "size" => "size in lines/columns or N%",
        "XxY" => "size as WIDTHxHEIGHT",
        "position" => "screen position (number or keyword)",
        "adjustment" => "amount to change by",
        "name" => "name",
        "new-name" => "new name to set",
        "window-name" => "window name",
        "session-name" => "session name",
        "layout-name" => "layout name or spec",
        "option" => "option name",
        "value" => "value to set",
        "state" => "on, off, or toggle",
        "sort-order" => "sort field (name, time, …)",
        "title" => "title text",
        "type" => "prompt / history type",
        "path" => "file path",
        "delay" => "delay in milliseconds",
        "duration" => "duration in milliseconds",
        "repeat-count" => "number of times to repeat",
        "border-lines" => "border style (single, double, heavy, …)",
        "border-style" => "border color / style",
        "style" => "style (fg=…, bg=…, attrs)",
        "channel" => "wait-channel name",
        "data" => "data to send",
        "end-line" => "last line to capture",
        "start-line" => "first line to capture",
        "hook" => "hook name",
        "inputs" => "comma-separated prompt answers",
        "prompt" => "prompt text",
        "prompts" => "comma-separated prompts",
        "match-string" => "string to match",
        "message" => "message text",
        "note" => "description note",
        "prefix-string" => "prefix string",
        "separator" => "separator string",
        "user" => "user name",
        "what" => "what to show",
        _ => return None,
    })
}

/// Description for a boolean flag letter, limited to meanings that are stable
/// across tmux commands. Ambiguous letters return `None` and are shown collapsed
/// on a single "flags" line rather than risk a wrong description.
fn flag_desc(letter: char) -> Option<&'static str> {
    Some(match letter {
        'Z' => "keep the pane zoomed",
        'd' => "detached — don't switch to it",
        'P' => "print info about the result",
        'a' => "all",
        'g' => "global",
        'q' => "quiet — suppress errors",
        _ => return None,
    })
}

// ---- usage parsing ------------------------------------------------------------

/// One argument of a command, as shown in the expanded help block.
#[derive(Clone)]
struct HelpParam {
    display: String,
    optional: bool,
    desc: Option<String>,
}

/// Split a usage fragment into top-level segments: `[bracketed]` groups (marked
/// optional, outer brackets stripped) and bare whitespace-separated words.
fn segments(s: &str) -> Vec<(String, bool)> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
        } else if chars[i] == '[' {
            let start = i;
            let mut depth = 0;
            while i < chars.len() {
                match chars[i] {
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            let inner: String = chars[start + 1..(i - 1).max(start + 1)].iter().collect();
            out.push((inner, true));
        } else {
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '[' {
                i += 1;
            }
            out.push((chars[start..i].iter().collect(), false));
        }
    }
    out
}

/// Classify a bracket-free fragment into params: `-x placeholder` options,
/// boolean flag clusters (described letters split out, the rest collapsed), and
/// bare positionals.
fn classify_flat(content: &str, optional: bool, out: &mut Vec<HelpParam>) {
    let toks: Vec<&str> = content.split_whitespace().collect();
    let mut i = 0;
    while i < toks.len() {
        let t = toks[i];
        if let Some(letters) = t.strip_prefix('-') {
            if i + 1 < toks.len() && !toks[i + 1].starts_with('-') {
                let arg = toks[i + 1];
                out.push(HelpParam {
                    display: format!("{} {}", t, arg),
                    optional,
                    desc: arg_desc(arg).map(str::to_string),
                });
                i += 2;
            } else {
                let mut unknown = String::new();
                for ch in letters.chars() {
                    match flag_desc(ch) {
                        Some(d) => out.push(HelpParam {
                            display: format!("-{}", ch),
                            optional,
                            desc: Some(d.to_string()),
                        }),
                        None => unknown.push(ch),
                    }
                }
                if !unknown.is_empty() {
                    out.push(HelpParam {
                        display: format!("-{}", unknown),
                        optional,
                        desc: Some("flags".to_string()),
                    });
                }
                i += 1;
            }
        } else {
            out.push(HelpParam {
                display: t.to_string(),
                optional,
                desc: arg_desc(t).map(str::to_string),
            });
            i += 1;
        }
    }
}

fn collect_params(s: &str, optional_ctx: bool, out: &mut Vec<HelpParam>) {
    for (content, bracketed) in segments(s) {
        let optional = optional_ctx || bracketed;
        if content.contains('[') {
            collect_params(&content, optional, out);
        } else {
            classify_flat(&content, optional, out);
        }
    }
}

/// Parse a `tmux list-commands` usage string into its ordered arguments.
fn parse_usage(usage: &str) -> Vec<HelpParam> {
    let mut out = Vec::new();
    collect_params(usage, false, &mut out);
    out
}

// ---- items --------------------------------------------------------------------

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

    // Needs args when anything survives stripping the optional `[...]` groups.
    let has_required = !parse_usage(usage).iter().all(|p| p.optional);
    let action = if has_required {
        Action::Fill(format!("{} ", name))
    } else {
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
        // Tab completes the command name into the input, ready for arguments.
        complete: Some(format!("{} ", name)),
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

/// A non-selectable help row carrying one parsed argument in `data`.
fn help_item(p: &HelpParam) -> Item {
    Item {
        selectable: Some(false),
        data: Some(Rc::new(p.clone())),
        ..Default::default()
    }
}

/// Rank commands by name + alias only. Matching against the usage text (the
/// item description) would make short queries subsequence-match far too many
/// commands (e.g. `rename-window` loosely matching `respawn-window`), which also
/// defeats the "exactly one command" help trigger.
fn match_commands(items: &[Item], query: &str) -> Vec<Item> {
    let stripped: Vec<Item> = items
        .iter()
        .map(|i| Item {
            description: None,
            ..i.clone()
        })
        .collect();
    default_filter(&stripped, query)
        .iter()
        .filter_map(|m| items.iter().find(|it| it.title == m.title).cloned())
        .collect()
}

/// Filter: fuzzy-rank the commands and always offer a "Run: <query>" row on top.
/// When exactly one command matches, drop its inline usage and expand its
/// arguments as help rows beneath it.
fn filter_commands(items: &[Item], query: &str) -> Vec<Item> {
    let matched = match_commands(items, query);
    let mut out = Vec::with_capacity(matched.len() + 1);
    out.push(run_typed_item(query));

    if matched.len() == 1 {
        let params = matched[0]
            .description
            .as_deref()
            .map(parse_usage)
            .unwrap_or_default();
        if !params.is_empty() {
            let mut cmd = matched[0].clone();
            cmd.description = None; // shown expanded below instead
            out.push(cmd);
            out.extend(params.iter().map(help_item));
            return out;
        }
    }

    out.extend(matched);
    out
}

// ---- rendering ----------------------------------------------------------------

fn pad(s: &str, width: i64) -> String {
    let gap = (width - display_width(s)).max(0) as usize;
    format!("{}{}", s, " ".repeat(gap))
}

/// Render a help row: `<indent><param>  <optional|required>  <description>`,
/// dim, with required args flagged in the accent color. Emits only foreground
/// color switches so the row background (set by the list composer) is preserved.
fn render_help_param(p: &HelpParam, colors: &Colors, _width: i64) -> String {
    let (tag, tag_color) = if p.optional {
        ("optional", &colors.muted)
    } else {
        ("required", &colors.accent)
    };
    format!(
        "       {}{}{}{}{}{}",
        colors.muted,
        pad(&p.display, 22),
        tag_color,
        pad(tag, 10),
        colors.muted,
        p.desc.clone().unwrap_or_default(),
    )
}

/// Row renderer: help rows get the dim expanded layout; everything else uses the
/// default item renderer.
pub fn render_item(item: &Item, ctx: &RenderItemCtx) -> String {
    match item
        .data
        .as_ref()
        .and_then(|d| d.downcast_ref::<HelpParam>())
    {
        Some(p) => render_help_param(p, ctx.colors, ctx.width),
        None => render_default_item(item, ctx.colors, ctx.active, ctx.width),
    }
}

pub fn command_prompt() -> PaletteDef {
    PaletteDef {
        title: Some("Run Command".to_string()),
        grouped: Some(false),
        empty_text: Some("Type a tmux command".to_string()),
        items: ItemsSource::Dynamic(Rc::new(build_items)),
        filter: Some(Rc::new(filter_commands)),
        render_item: Some(Rc::new(render_item)),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(usage: &str) -> Vec<(String, bool, Option<String>)> {
        parse_usage(usage)
            .into_iter()
            .map(|p| (p.display, p.optional, p.desc))
            .collect()
    }

    #[test]
    fn parses_option_with_arg_and_required_positional() {
        let got = params("[-t target-window] new-name");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, "-t target-window");
        assert!(got[0].1); // optional
        assert_eq!(
            got[0].2.as_deref(),
            Some("window to target (default: current)")
        );
        assert_eq!(got[1].0, "new-name");
        assert!(!got[1].1); // required
        assert_eq!(got[1].2.as_deref(), Some("new name to set"));
    }

    #[test]
    fn splits_described_flags_and_collapses_the_rest() {
        // -Z and -d are known; -b -f -h -I -v collapse to one "flags" line.
        let got = params("[-bdfhIvPZ]");
        let displays: Vec<&str> = got.iter().map(|p| p.0.as_str()).collect();
        assert!(displays.contains(&"-Z"));
        assert!(displays.contains(&"-d"));
        assert!(displays.contains(&"-P"));
        // The undescribed remainder is a single collapsed line, in usage order.
        assert!(displays.contains(&"-bfhIv"));
        assert!(got.iter().all(|p| p.1)); // all optional
    }

    #[test]
    fn handles_nested_optional_groups() {
        // bind-key: `[-nr] [-T key-table] [-N note] key [command [arguments]]`
        let got = params("[-nr] [-T key-table] [-N note] key [command [arguments]]");
        let displays: Vec<&str> = got.iter().map(|p| p.0.as_str()).collect();
        assert!(displays.contains(&"-T key-table"));
        assert!(displays.contains(&"key")); // required positional
        assert!(displays.contains(&"command")); // from the nested group
        assert!(displays.contains(&"arguments"));
        // `key` is required; the nested `command`/`arguments` stay optional.
        let key = got.iter().find(|p| p.0 == "key").unwrap();
        assert!(!key.1);
        let cmd = got.iter().find(|p| p.0 == "command").unwrap();
        assert!(cmd.1);
    }

    #[test]
    fn all_optional_usage_yields_a_runnable_command() {
        let item = parse_command_line(
            "detach-client (detach) [-aP] [-E shell-command] [-t target-client]",
        )
        .unwrap();
        assert!(matches!(item.action, Action::Tmux(ref c) if c == "detach-client"));
        assert_eq!(item.complete.as_deref(), Some("detach-client "));
    }

    #[test]
    fn required_arg_command_completes_on_enter() {
        let item =
            parse_command_line("rename-window (renamew) [-t target-window] new-name").unwrap();
        assert!(matches!(item.action, Action::Fill(ref t) if t == "rename-window "));
    }

    #[test]
    fn single_match_expands_help_rows() {
        let items = vec![parse_command_line("rename-window [-t target-window] new-name").unwrap()];
        let vis = filter_commands(&items, "rename-window");
        assert_eq!(vis[0].title, "Run: rename-window");
        assert_eq!(vis[1].title, "rename-window");
        assert_eq!(vis[1].description, None); // inline usage dropped
                                              // Help rows follow, non-selectable, carrying HelpParam data.
        assert!(vis.len() >= 4);
        assert_eq!(vis[2].selectable, Some(false));
        assert!(vis[2]
            .data
            .as_ref()
            .and_then(|d| d.downcast_ref::<HelpParam>())
            .is_some());
    }

    #[test]
    fn multiple_matches_do_not_expand() {
        let items = vec![
            parse_command_line("kill-pane (killp) [-a] [-t target-pane]").unwrap(),
            parse_command_line("kill-window (killw) [-a] [-t target-window]").unwrap(),
        ];
        let vis = filter_commands(&items, "kill");
        assert_eq!(vis[0].title, "Run: kill");
        // Both commands present, none marked non-selectable (no help rows).
        assert!(vis[1..].iter().all(|i| i.selectable != Some(false)));
    }

    #[test]
    fn matching_ignores_usage_text_so_full_names_narrow_to_one() {
        let items = vec![
            parse_command_line("rename-window (renamew) [-t target-window] new-name").unwrap(),
            parse_command_line(
                "respawn-window (respawnw) [-k] [-c start-directory] [-t target-window]",
            )
            .unwrap(),
            parse_command_line("new-window (neww) [-t target-window] [shell-command]").unwrap(),
        ];
        // Without stripping the usage, "rename-window" subsequence-matches the
        // others (they share "target-window"); name-only matching narrows to one.
        let hits = match_commands(&items, "rename-window");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "rename-window");
        // ...and the single match expands into help rows.
        let vis = filter_commands(&items, "rename-window");
        assert!(vis.iter().any(|i| i
            .data
            .as_ref()
            .is_some_and(|d| d.downcast_ref::<HelpParam>().is_some())));
    }

    #[test]
    fn run_row_dispatches_arbitrary_input() {
        let items = vec![parse_command_line("kill-pane [-t target-pane]").unwrap()];
        let vis = filter_commands(&items, "new-session -s work");
        assert!(matches!(&vis[0].action, Action::Tmux(c) if c == "new-session -s work"));
    }
}
