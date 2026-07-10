//! `command-prompt` palette — a fuzzy replacement for tmux's `prefix + :`
//! command prompt.
//!
//! The command list is pulled live from `tmux list-commands`, so *every* tmux
//! command is searchable by name and alias. Selecting a command runs it
//! immediately when it needs no arguments, or completes `<name> ` into the input
//! when it takes arguments. Tab cycles through the matches (see the palette
//! loop), and a synthetic "Run: <what you typed>" row always dispatches the
//! typed line via the after-popup trick that keeps interactive prompts fed.
//!
//! Each row says what its command *does*, from the bundled `CMD_DESCS` table —
//! tmux itself offers no such text, and the usage string it does offer is no
//! substitute (`new-pane`'s runs to 269 characters). The usage rides along in
//! `Item.data` instead, and comes into its own once the first word is a complete
//! command name (or alias): that command's arguments are then expanded below it,
//! one per line, tagged optional/required with a short description drawn from a
//! bundled glossary of tmux's (very consistent) placeholder names and common
//! flags. The resting list is grouped by topic (sessions, windows, panes, …) and
//! each row carries a nerd-font icon for its verb (new, kill, rename, …), so all
//! ~90 commands stay browsable.

use std::rc::Rc;

use crate::fuzzy::default_filter;
use crate::render::render_default_item;
use crate::text::display_width;
use crate::tmux::tmux;
use crate::types::{Action, Colors, Item, ItemsSource, PaletteDef, RenderItemCtx};

/// Marker for the synthetic row that runs exactly what the user typed.
const RUN_ICON: &str = "󰐊"; // md-play
/// Fallback for a command whose verb is missing from the tables below — e.g. one
/// added by a newer tmux than this build knows about.
const CMD_ICON: &str = "󰅂"; // md-chevron_right

// ---- icons -------------------------------------------------------------------
//
// Nerd-font (Material Design) glyphs, the same vocabulary as the main palette.
// The resting list is grouped by topic, so the noun is already on screen in the
// category header — the icon carries the *verb*, which is what varies within a
// group and what you actually scan for.

/// Commands whose leading verb would mislead (`copy-mode` is a mode, not a copy
/// action) or that have no verb worth grouping on. Checked before `VERB_ICONS`.
const NAME_ICONS: &[(&str, &str)] = &[
    ("attach-session", "󰚥"),  // md-power_plug
    ("break-pane", "󰘖"),      // md-arrow_expand
    ("capture-pane", "󰄀"),    // md-camera
    ("clock-mode", "󰅐"),      // md-clock_outline
    ("command-prompt", "󰞷"),  // md-console_line
    ("confirm-before", "󰋗"),  // md-help_circle
    ("copy-mode", "󰆏"),       // md-content_copy
    ("customize-mode", "󰒓"),  // md-cog
    ("detach-client", "󰍃"),   // md-logout
    ("display-menu", "󰍜"),    // md-menu
    ("display-message", "󰍡"), // md-message
    ("display-panes", "󰎠"),   // md-numeric
    ("display-popup", "󱂬"),   // md-dock_window
    ("has-session", "󰄬"),     // md-check
    ("if-shell", "󰘬"),        // md-source_branch
    ("join-pane", "󰃸"),       // md-call_merge
    ("link-window", "󰌷"),     // md-link
    ("pipe-pane", "󰟥"),       // md-pipe
    ("rotate-window", "󰑧"),   // md-rotate_right
    ("run-shell", "󰆍"),       // md-console
    ("server-access", "󰢏"),   // md-shield_account
    ("source-file", "󰈙"),     // md-file_document
    // Escaped, not literal: this codicon lives in the BMP private-use area, which
    // editors and pipes silently strip. The MDI glyphs above are outside it.
    ("split-window", "\u{eb56}"), // cod-split_horizontal
    ("start-server", "󰐊"),        // md-play
    ("suspend-client", "󰏤"),      // md-pause
    ("switch-client", "󰀙"),       // md-account_switch
    ("unbind-key", "󰌐"),          // md-keyboard_off
    ("unlink-window", "󰌸"),       // md-link_off
    ("wait-for", "󰔟"),            // md-timer_sand
];

/// Icons keyed by a command's leading verb — tmux names are `verb-noun`, so this
/// covers every command the pins above don't.
const VERB_ICONS: &[(&str, &str)] = &[
    ("bind", "󰌌"),     // md-keyboard
    ("choose", "󰍉"),   // md-magnify
    ("clear", "󰃢"),    // md-broom
    ("delete", "󰆴"),   // md-delete
    ("find", "󰍉"),     // md-magnify
    ("kill", "󰆴"),     // md-delete
    ("last", "󰋚"),     // md-history
    ("list", "󰉹"),     // md-format_list_bulleted
    ("load", "󰇚"),     // md-download
    ("lock", "󰌾"),     // md-lock
    ("move", "󰁁"),     // md-arrow_all
    ("new", "󰐕"),      // md-plus
    ("next", "󰁔"),     // md-arrow_right
    ("paste", "󰆒"),    // md-content_paste
    ("previous", "󰁍"), // md-arrow_left
    ("refresh", "󰑓"),  // md-reload
    ("rename", "󰏫"),   // md-pencil
    ("resize", "󰩨"),   // md-resize
    ("respawn", "󰑓"),  // md-reload
    ("save", "󰆓"),     // md-content_save
    ("select", "󰆣"),   // md-crosshairs
    ("send", "󰒊"),     // md-send
    ("set", "󰒓"),      // md-cog
    ("show", "󰈈"),     // md-eye
    ("swap", "󰓡"),     // md-swap_horizontal
];

/// Pick a command's icon: exact name first, then its leading verb, then a
/// neutral marker.
fn icon_for(name: &str) -> &'static str {
    if let Some((_, icon)) = NAME_ICONS.iter().find(|(n, _)| *n == name) {
        return icon;
    }
    let verb = name.split('-').next().unwrap_or("");
    VERB_ICONS
        .iter()
        .find(|(v, _)| *v == verb)
        .map_or(CMD_ICON, |(_, icon)| *icon)
}

// ---- descriptions --------------------------------------------------------------

/// What each command does, in the imperative. tmux publishes no such text —
/// `list-commands` gives only `name (alias) usage` — so it is bundled here.
///
/// Kept under 40 cells: the widest command name and alias chip together take 39
/// of the 94-cell body at the default popup width, so every row still fits with
/// room to spare. A row that overflows is worse than one that is terse — the
/// renderer clips it, and used to strip the whole row's styling doing so.
///
/// `join-pane` and `move-pane` read alike because they *are* alike: identical
/// usage, and joining within one window works for both. Describing them
/// differently would imply a distinction tmux does not make.
#[rustfmt::skip]
const CMD_DESCS: &[(&str, &str)] = &[
    ("attach-session",       "attach to a session"),
    ("bind-key",             "bind a key to a command"),
    ("break-pane",           "move a pane out into its own window"),
    ("capture-pane",         "copy the pane's screen to a buffer"),
    ("choose-buffer",        "browse the paste buffers"),
    ("choose-client",        "browse the attached clients"),
    ("choose-tree",          "browse sessions, windows and panes"),
    ("clear-history",        "clear the pane's scrollback"),
    ("clear-prompt-history", "clear the command prompt history"),
    ("clock-mode",           "show a clock in the pane"),
    ("command-prompt",       "prompt for a tmux command"),
    ("confirm-before",       "ask before running a command"),
    ("copy-mode",            "enter copy mode to scroll and select"),
    ("customize-mode",       "browse and edit options interactively"),
    ("delete-buffer",        "delete a paste buffer"),
    ("detach-client",        "detach a client from its session"),
    ("display-menu",         "show a menu of commands"),
    ("display-message",      "show a message in the status line"),
    ("display-panes",        "show pane numbers to pick one"),
    ("display-popup",        "run a command in a popup window"),
    ("find-window",          "search windows by name or content"),
    ("has-session",          "check whether a session exists"),
    ("if-shell",             "run a command if a shell test passes"),
    ("join-pane",            "move a pane in beside another pane"),
    ("kill-pane",            "close a pane"),
    ("kill-server",          "stop the server and all sessions"),
    ("kill-session",         "destroy a session and its windows"),
    ("kill-window",          "close a window and its panes"),
    ("last-pane",            "focus the previously active pane"),
    ("last-window",          "switch back to the last window"),
    ("link-window",          "link a window into another session"),
    ("list-buffers",         "list the paste buffers"),
    ("list-clients",         "list clients attached to a session"),
    ("list-commands",        "list every tmux command"),
    ("list-keys",            "list the key bindings"),
    ("list-panes",           "list panes in a window or session"),
    ("list-sessions",        "list the server's sessions"),
    ("list-windows",         "list a session's windows"),
    ("load-buffer",          "load a buffer from a file"),
    ("lock-client",          "lock a single client"),
    ("lock-server",          "lock every client"),
    ("lock-session",         "lock every client of a session"),
    ("move-pane",            "move a pane in beside another pane"),
    ("move-window",          "move a window to another index"),
    ("new-pane",             "create a floating pane"),
    ("new-session",          "create a session"),
    ("new-window",           "create a window"),
    ("next-layout",          "cycle to the next layout"),
    ("next-window",          "switch to the next window"),
    ("paste-buffer",         "paste a buffer into the pane"),
    ("pipe-pane",            "pipe the pane's output to a command"),
    ("previous-layout",      "cycle to the previous layout"),
    ("previous-window",      "switch to the previous window"),
    ("refresh-client",       "redraw a client"),
    ("rename-session",       "rename a session"),
    ("rename-window",        "rename a window"),
    ("resize-pane",          "resize a pane"),
    ("resize-window",        "resize a window"),
    ("respawn-pane",         "restart the pane's command"),
    ("respawn-window",       "restart the window's command"),
    ("rotate-window",        "rotate the panes within the window"),
    ("run-shell",            "run a shell command"),
    ("save-buffer",          "write a buffer to a file"),
    ("select-layout",        "apply a pane layout"),
    ("select-pane",          "focus a pane"),
    ("select-window",        "switch to a window"),
    ("send-keys",            "send keys to a pane as if typed"),
    ("send-prefix",          "send the prefix key to the pane"),
    ("server-access",        "grant or revoke another user's access"),
    ("set-buffer",           "set a paste buffer's contents"),
    ("set-environment",      "set an environment variable"),
    ("set-hook",             "run a command when an event fires"),
    ("set-option",           "set a session or server option"),
    ("set-window-option",    "set a window option"),
    ("show-buffer",          "print a buffer's contents"),
    ("show-environment",     "show the environment"),
    ("show-hooks",           "show the hooks that are set"),
    ("show-messages",        "show the server's message log"),
    ("show-options",         "show option values"),
    ("show-prompt-history",  "show the command prompt history"),
    ("show-window-options",  "show window option values"),
    ("source-file",          "run commands from a file"),
    ("split-window",         "split the pane in two"),
    ("start-server",         "start the tmux server"),
    ("suspend-client",       "suspend a client to the shell"),
    ("swap-pane",            "exchange two panes"),
    ("swap-window",          "exchange two windows"),
    ("switch-client",        "point a client at another session"),
    ("unbind-key",           "remove a key binding"),
    ("unlink-window",        "remove a linked window"),
    ("wait-for",             "block or signal on a channel"),
];

/// `None` for a command a newer tmux added that this build has never heard of;
/// such a row falls back to showing its usage, as every row once did.
fn desc_for(name: &str) -> Option<&'static str> {
    CMD_DESCS
        .binary_search_by_key(&name, |(n, _)| n)
        .ok()
        .map(|i| CMD_DESCS[i].1)
}

// ---- categories --------------------------------------------------------------

/// Topic groups for the resting list, in display order.
const CATEGORIES: &[&str] = &[
    "Sessions",
    "Windows",
    "Panes",
    "Copy & Buffers",
    "Key Bindings",
    "Options & Hooks",
    "Display & Prompts",
    "Server & Misc",
];

/// Bucket a command into one of `CATEGORIES` from its name. tmux names are
/// `verb-noun`, so the noun usually decides the group; a handful of commands
/// whose keyword would misfile them (or that have none) are pinned explicitly.
fn category_for(name: &str) -> &'static str {
    match name {
        // split-window makes a pane; clear-history clears a pane's scrollback.
        "split-window" | "clear-history" => return "Panes",
        "copy-mode" => return "Copy & Buffers",
        "customize-mode" => return "Options & Hooks",
        "command-prompt"
        | "confirm-before"
        | "clock-mode"
        | "choose-tree"
        | "clear-prompt-history"
        | "show-prompt-history" => return "Display & Prompts",
        "if-shell" | "run-shell" | "list-commands" | "source-file" | "wait-for"
        | "show-messages" => return "Server & Misc",
        _ => {}
    }
    if name.starts_with("display-") {
        return "Display & Prompts";
    }
    // Keyword scan, most specific first (so `set-window-option` files under
    // options, not windows).
    const KEYWORDS: &[(&str, &str)] = &[
        ("buffer", "Copy & Buffers"),
        ("option", "Options & Hooks"),
        ("hook", "Options & Hooks"),
        ("environment", "Options & Hooks"),
        ("pane", "Panes"),
        ("layout", "Windows"),
        ("window", "Windows"),
        ("session", "Sessions"),
        ("client", "Sessions"),
        ("prefix", "Key Bindings"),
        ("key", "Key Bindings"),
        ("server", "Server & Misc"),
    ];
    for (kw, cat) in KEYWORDS {
        if name.contains(kw) {
            return cat;
        }
    }
    "Server & Misc"
}

/// Position of a category in `CATEGORIES` (used to order the resting list).
fn category_rank(cat: &str) -> usize {
    CATEGORIES
        .iter()
        .position(|c| *c == cat)
        .unwrap_or(CATEGORIES.len())
}

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

/// A command's raw usage string, carried in `Item.data` rather than on the row.
/// It is what the argument help is parsed from, but it runs to 269 characters
/// for `new-pane` — far past the row — so the row shows the description instead.
struct CommandMeta {
    usage: String,
}

/// The usage string stashed on a command item, or `""` for a row that carries
/// none (the synthetic "Run:" row, or a help row).
fn usage_of(item: &Item) -> &str {
    item.data
        .as_ref()
        .and_then(|d| d.downcast_ref::<CommandMeta>())
        .map_or("", |m| m.usage.as_str())
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

    // Needs args when anything survives stripping the optional `[...]` groups.
    let has_required = !parse_usage(usage).iter().all(|p| p.optional);
    let action = if has_required {
        Action::Fill(format!("{} ", name))
    } else {
        Action::Tmux(name.to_string())
    };

    Some(Item {
        icon: Some(icon_for(name).to_string()),
        title: name.to_string(),
        description: desc_for(name)
            .map(str::to_string)
            .or_else(|| (!usage.is_empty()).then(|| usage.to_string())),
        aliases: alias.map(|a| vec![a]),
        action,
        category: Some(category_for(name).to_string()),
        // Tab completes the command name into the input, ready for arguments.
        complete: Some(format!("{} ", name)),
        data: Some(Rc::new(CommandMeta {
            usage: usage.to_string(),
        })),
        ..Default::default()
    })
}

/// Every tmux command, straight from `tmux list-commands`, ordered by topic so
/// the grouped resting list reads top to bottom without repeating headers.
fn build_items() -> Vec<Item> {
    let mut items: Vec<Item> = tmux(&["list-commands"])
        .lines()
        .filter_map(parse_command_line)
        .collect();
    items.sort_by(|a, b| {
        let ra = category_rank(a.category.as_deref().unwrap_or(""));
        let rb = category_rank(b.category.as_deref().unwrap_or(""));
        ra.cmp(&rb).then_with(|| a.title.cmp(&b.title))
    });
    items
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

/// Rank commands by name + alias only. Matching against the description or the
/// topic (the category) would make short queries subsequence-match far too many
/// commands — e.g. `rename-window` loosely matching `respawn-window`, or `copy`
/// matching everything filed under "Copy & Buffers". Searching the descriptions
/// would be genuinely useful (`close` finding the `kill-*` commands), but wants
/// a boundary-anchored match rather than the subsequence one, so it stays out.
fn match_commands(items: &[Item], query: &str) -> Vec<Item> {
    let stripped: Vec<Item> = items
        .iter()
        .map(|i| Item {
            description: None,
            category: None,
            ..i.clone()
        })
        .collect();
    default_filter(&stripped, query)
        .iter()
        .filter_map(|m| items.iter().find(|it| it.title == m.title).cloned())
        .collect()
}

/// True when `head` is exactly this command's name or one of its aliases.
fn command_named(item: &Item, head: &str) -> bool {
    item.title == head
        || item
            .aliases
            .as_ref()
            .is_some_and(|a| a.iter().any(|x| x == head))
}

/// Filter: always offer a "Run: <query>" row on top. Once the first word is a
/// complete command name (you've typed `<command>` and are onto its
/// parameters), show that command — description and all — and expand its
/// arguments as help rows, which stay put while you type the params. Otherwise
/// fuzzy-rank the command list by name so a partial word just narrows the
/// choices without prematurely committing to one.
fn filter_commands(items: &[Item], query: &str) -> Vec<Item> {
    let mut out = Vec::new();
    out.push(run_typed_item(query));

    let head = query.split_whitespace().next().unwrap_or("");
    if let Some(cmd) = items.iter().find(|it| command_named(it, head)) {
        let params = parse_usage(usage_of(cmd));
        out.push(cmd.clone());
        out.extend(params.iter().map(help_item));
        return out;
    }

    out.extend(match_commands(items, query));
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
        grouped: Some(true),
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
    fn exact_command_expands_help_rows() {
        let items = vec![parse_command_line("rename-window [-t target-window] new-name").unwrap()];
        let vis = filter_commands(&items, "rename-window");
        assert_eq!(vis[0].title, "Run: rename-window");
        assert_eq!(vis[1].title, "rename-window");
        // The command keeps its description while you type parameters; the usage
        // it was parsed from rides in `data`, not on the row.
        assert_eq!(vis[1].description.as_deref(), Some("rename a window"));
        assert_eq!(usage_of(&vis[1]), "[-t target-window] new-name");
        // Help rows follow, non-selectable, carrying HelpParam data.
        assert!(vis.len() >= 4);
        assert_eq!(vis[2].selectable, Some(false));
        assert!(vis[2]
            .data
            .as_ref()
            .and_then(|d| d.downcast_ref::<HelpParam>())
            .is_some());
    }

    fn has_help_rows(vis: &[Item]) -> bool {
        vis.iter().any(|i| {
            i.data
                .as_ref()
                .is_some_and(|d| d.downcast_ref::<HelpParam>().is_some())
        })
    }

    #[test]
    fn partial_name_stays_a_list_but_exact_name_plus_params_expands() {
        let items = vec![
            parse_command_line("rename-window (renamew) [-t target-window] new-name").unwrap(),
        ];
        // Partial name: no help rows, just the ranked command under the Run row.
        assert!(!has_help_rows(&filter_commands(&items, "rename-w")));
        // Exact name alone: help expands (you've committed to the command).
        assert!(has_help_rows(&filter_commands(&items, "rename-window")));
        // Exact name plus a typed parameter: help stays put.
        let typing = filter_commands(&items, "rename-window my-name");
        assert_eq!(typing[0].title, "Run: rename-window my-name");
        assert!(has_help_rows(&typing));
        // The alias triggers it the same way.
        assert!(has_help_rows(&filter_commands(&items, "renamew foo")));
    }

    #[test]
    fn matching_ignores_category_so_topic_words_do_not_broaden() {
        let items =
            vec![parse_command_line("paste-buffer (pasteb) [-p] [-b buffer-name]").unwrap()];
        // paste-buffer files under "Copy & Buffers"; "copy" (absent from its
        // name/alias) must not match it via the category text.
        assert!(match_commands(&items, "copy").is_empty());
    }

    #[test]
    fn categorizes_commands_by_topic() {
        assert_eq!(category_for("set-window-option"), "Options & Hooks");
        assert_eq!(category_for("split-window"), "Panes");
        assert_eq!(category_for("rename-window"), "Windows");
        assert_eq!(category_for("kill-pane"), "Panes");
        assert_eq!(category_for("attach-session"), "Sessions");
        assert_eq!(category_for("paste-buffer"), "Copy & Buffers");
        assert_eq!(category_for("bind-key"), "Key Bindings");
        assert_eq!(category_for("display-popup"), "Display & Prompts");
        assert_eq!(category_for("choose-tree"), "Display & Prompts");
        assert_eq!(category_for("run-shell"), "Server & Misc");
    }

    /// Every command `tmux list-commands` prints (tmux 3.7).
    #[rustfmt::skip]
    const ALL_COMMANDS: &[&str] = &[
        "attach-session","bind-key","break-pane","capture-pane","choose-buffer","choose-client",
        "choose-tree","clear-history","clear-prompt-history","clock-mode","command-prompt",
        "confirm-before","copy-mode","customize-mode","delete-buffer","detach-client","display-menu",
        "display-message","display-panes","display-popup","find-window","has-session","if-shell",
        "join-pane","kill-pane","kill-server","kill-session","kill-window","last-pane","last-window",
        "link-window","list-buffers","list-clients","list-commands","list-keys","list-panes",
        "list-sessions","list-windows","load-buffer","lock-client","lock-server","lock-session",
        "move-pane","move-window","new-pane","new-session","new-window","next-layout","next-window",
        "paste-buffer","pipe-pane","previous-layout","previous-window","refresh-client",
        "rename-session","rename-window","resize-pane","resize-window","respawn-pane",
        "respawn-window","rotate-window","run-shell","save-buffer","select-layout","select-pane",
        "select-window","send-keys","send-prefix","server-access","set-buffer","set-environment",
        "set-hook","set-option","set-window-option","show-buffer","show-environment","show-hooks",
        "show-messages","show-options","show-prompt-history","show-window-options","source-file",
        "split-window","start-server","suspend-client","swap-pane","swap-window","switch-client",
        "unbind-key","unlink-window","wait-for",
    ];

    #[test]
    fn every_tmux_command_has_a_real_icon() {
        for name in ALL_COMMANDS {
            let icon = icon_for(name);
            assert_ne!(icon, CMD_ICON, "{name} fell back to the generic icon");
            assert!(!icon.is_empty(), "{name} has an empty icon");
        }
    }

    #[test]
    fn icons_come_from_the_verb_unless_pinned() {
        // Verb-keyed: the noun is already in the category header.
        assert_eq!(icon_for("kill-pane"), icon_for("kill-window"));
        assert_eq!(icon_for("new-session"), icon_for("new-window"));
        assert_ne!(icon_for("new-window"), icon_for("kill-window"));
        // Pinned names win over their (misleading) leading verb.
        assert_eq!(icon_for("copy-mode"), "󰆏");
        assert_ne!(icon_for("set-option"), icon_for("source-file"));
        // An unknown future command still renders something.
        assert_eq!(icon_for("teleport-pane"), CMD_ICON);
    }

    #[test]
    fn built_items_carry_their_icon() {
        let item = parse_command_line("kill-pane (killp) [-a] [-t target-pane]").unwrap();
        assert_eq!(item.icon.as_deref(), Some(icon_for("kill-pane")));
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

    // ---- descriptions ----------------------------------------------------------

    /// `desc_for` binary-searches, so the table has to stay ordered — and the
    /// entries have to be commands tmux actually has.
    #[test]
    fn description_table_is_sorted_and_names_real_commands() {
        let names: Vec<&str> = CMD_DESCS.iter().map(|(n, _)| *n).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "CMD_DESCS must be sorted by command name");
        for name in &names {
            assert!(ALL_COMMANDS.contains(name), "{name} is not a tmux command");
        }
    }

    #[test]
    fn every_tmux_command_has_a_description() {
        for name in ALL_COMMANDS {
            assert!(desc_for(name).is_some(), "{name} has no description");
        }
    }

    /// The row must fit the 94-cell body of a default 100-column popup, or the
    /// renderer clips it. The prefix is marker + icon + two spaces + name +
    /// alias chip; the description then costs ` - ` plus its own width.
    #[test]
    fn every_row_fits_the_default_popup() {
        for (name, desc) in CMD_DESCS {
            assert!(
                display_width(desc) < 40,
                "{name}: description is {} cells",
                display_width(desc)
            );
        }
        // Worst case in practice: the longest name paired with its alias chip.
        let widest = "clear-prompt-history";
        let prefix = 1 + 1 + 1 + 2; // marker, gap, icon, two gaps
        let chip = 4 + display_width("clearphist");
        let desc = desc_for(widest).unwrap();
        let row = prefix + display_width(widest) + chip + 3 + display_width(desc);
        assert!(row <= 94, "{widest} renders {row} cells wide");
    }

    #[test]
    fn rows_show_prose_and_keep_the_usage_for_the_help_block() {
        let item = parse_command_line("kill-pane (killp) [-a] [-t target-pane]").unwrap();
        assert_eq!(item.description.as_deref(), Some("close a pane"));
        assert_eq!(usage_of(&item), "[-a] [-t target-pane]");
    }

    /// A command from a tmux newer than this build still says something useful —
    /// its usage, exactly as every row showed before `CMD_DESCS` existed.
    #[test]
    fn unknown_commands_fall_back_to_their_usage() {
        let item =
            parse_command_line("teleport-pane (telep) [-t target-pane] destination").unwrap();
        assert_eq!(
            item.description.as_deref(),
            Some("[-t target-pane] destination")
        );
    }

    /// `join-pane` and `move-pane` are the same command in tmux — identical
    /// usage, and joining within one window works for both.
    #[test]
    fn join_and_move_pane_read_alike() {
        assert_eq!(desc_for("join-pane"), desc_for("move-pane"));
    }
}
