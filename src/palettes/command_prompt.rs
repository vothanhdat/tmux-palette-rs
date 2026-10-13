//! `command-prompt` palette — a fuzzy replacement for tmux's `prefix + :`
//! command prompt.
//!
//! The command list is pulled live from `tmux list-commands`, so *every* tmux
//! command is searchable by name and alias. Selecting a command runs it
//! immediately when it needs no arguments, or completes `<name> ` into the input
//! when it takes arguments. Tab completes the highlighted match and cycles on
//! from there (see the palette loop), and a synthetic "Run: <what you typed>"
//! row always dispatches the typed line via the after-popup trick that keeps
//! interactive prompts fed.
//!
//! The space after a command name commits to it: the name becomes a pill in
//! the input that Backspace deletes whole, and the list gives way to that
//! command. Until then an exact name or alias only narrows the list — `rename`
//! is `rename-session`'s alias, but you may be typing `rename-window`.
//!
//! Each row says what its command *does*, from the bundled `CMD_DESCS` table —
//! tmux itself offers no such text, and the usage string it does offer is no
//! substitute (`new-pane`'s runs to 269 characters). The usage rides along in
//! `Item.data` instead, and comes into its own once the input commits to a
//! command name (or alias): that command's arguments are then expanded below it,
//! one per line, tagged optional/required with a short description drawn from a
//! bundled glossary of tmux's (very consistent) placeholder names and common
//! flags. The resting list is grouped by topic (sessions, windows, panes, …) and
//! each row carries a nerd-font icon for its verb (new, kill, rename, …), so all
//! ~90 commands stay browsable.

use std::collections::HashMap;
use std::rc::Rc;

use super::command_help::{help_for, CommandHelp};
use crate::fuzzy::default_filter;
use crate::render::render_default_item;
use crate::text::{clip, display_width};
use crate::tmux::tmux;
use crate::types::{Action, Colors, Item, ItemsSource, PaletteDef, PreviewCtx, RenderItemCtx};

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
    ("switch-mode", "󰍉"),         // md-magnify
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
    ("switch-mode",          "fuzzy-pick a session or window"),
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
        | "switch-mode"
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
        "arguments" | "argument" => "arguments for the command",
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
    /// The flag letter, for a single flag (`-h`, `-t target-pane`).
    flag: Option<char>,
    /// The flag already appears in the typed arguments.
    used: bool,
    /// Cells the display column takes: the command's widest param, plus a gap.
    col: i64,
}

impl HelpParam {
    fn new(display: String, optional: bool, desc: Option<String>, flag: Option<char>) -> Self {
        HelpParam {
            display,
            optional,
            desc,
            flag,
            used: false,
            col: 0,
        }
    }
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
/// bare positionals. Descriptions come from the command's own help first, then
/// the generic glossary.
fn classify_flat(
    help: Option<&CommandHelp>,
    content: &str,
    optional: bool,
    out: &mut Vec<HelpParam>,
) {
    let flag_text = |letter: char| help.and_then(|h| h.flag(letter));
    let toks: Vec<&str> = content.split_whitespace().collect();
    let mut i = 0;
    while i < toks.len() {
        let t = toks[i];
        if t.starts_with('-') && t.contains('|') {
            // `-L|-S|-U`: alternatives, each a flag of its own.
            for alt in t.split('|') {
                classify_flat(help, alt, optional, out);
            }
            i += 1;
        } else if let Some(letters) = t.strip_prefix('-') {
            if i + 1 < toks.len() && !toks[i + 1].starts_with('-') {
                let arg = toks[i + 1];
                let letter = letters.chars().next();
                let desc = letter.and_then(flag_text).or_else(|| arg_desc(arg));
                out.push(HelpParam::new(
                    format!("{} {}", t, arg),
                    optional,
                    desc.map(str::to_string),
                    letter,
                ));
                i += 2;
            } else {
                let mut unknown = String::new();
                for ch in letters.chars() {
                    match flag_text(ch).or_else(|| flag_desc(ch)) {
                        Some(d) => out.push(HelpParam::new(
                            format!("-{}", ch),
                            optional,
                            Some(d.to_string()),
                            Some(ch),
                        )),
                        None => unknown.push(ch),
                    }
                }
                if !unknown.is_empty() {
                    out.push(HelpParam::new(
                        format!("-{}", unknown),
                        optional,
                        Some("flags".to_string()),
                        None,
                    ));
                }
                i += 1;
            }
        } else if t == "..." && out.last().is_some_and(|p| p.flag.is_none()) {
            // `argument ...`: one repeatable positional, not two.
            if let Some(last) = out.last_mut() {
                last.display.push_str(" ...");
            }
            i += 1;
        } else {
            let desc = help.and_then(|h| h.arg(t)).or_else(|| arg_desc(t));
            out.push(HelpParam::new(
                t.to_string(),
                optional,
                desc.map(str::to_string),
                None,
            ));
            i += 1;
        }
    }
}

fn collect_params(
    help: Option<&CommandHelp>,
    s: &str,
    optional_ctx: bool,
    out: &mut Vec<HelpParam>,
) {
    for (content, bracketed) in segments(s) {
        let optional = optional_ctx || bracketed;
        if content.contains('[') {
            collect_params(help, &content, optional, out);
        } else {
            classify_flat(help, &content, optional, out);
        }
    }
}

/// Parse command `name`'s `tmux list-commands` usage string into its ordered
/// arguments.
fn parse_usage(name: &str, usage: &str) -> Vec<HelpParam> {
    let help = help_for(name);
    let mut out = Vec::new();
    collect_params(help, usage, false, &mut out);
    // tmux lists some letters twice — split-window's `[-bdefhIPvZ]` and its
    // `[-e environment]` — so drop the bare flag that has a valued twin.
    let valued: Vec<char> = out
        .iter()
        .filter(|p| p.display.contains(' ') && p.display.starts_with('-'))
        .filter_map(|p| p.flag)
        .collect();
    out.retain(|p| p.display.contains(' ') || !p.flag.is_some_and(|f| valued.contains(&f)));
    for p in out
        .iter_mut()
        .filter(|p| p.flag.is_none() && p.display.starts_with('-'))
    {
        p.display.retain(|c| !valued.contains(&c));
    }
    out.retain(|p| p.display != "-");
    let col = out
        .iter()
        .map(|p| display_width(&p.display))
        .max()
        .unwrap_or(0)
        + 2;
    for p in &mut out {
        p.col = col;
    }
    out
}

/// Flag letters already typed after the command name: `-h`, and clusters such
/// as `-dh`. `--` and anything after it are arguments, not flags.
fn typed_flags(args: &str) -> Vec<char> {
    let mut out = Vec::new();
    for tok in args.split_whitespace() {
        if tok == "--" {
            break;
        }
        if let Some(letters) = tok.strip_prefix('-') {
            out.extend(letters.chars().filter(char::is_ascii_alphanumeric));
        }
    }
    out
}

// ---- items --------------------------------------------------------------------

/// What a command row carries in `Item.data`: its raw usage string — what the
/// argument help is parsed from, but at up to 269 characters (`new-pane`) far
/// too long for the row, which shows the description instead — and the keys
/// bound to it, for the details strip.
struct CommandMeta {
    name: String,
    usage: String,
    bindings: Vec<Binding>,
}

/// A key bound to a command, as the details strip shows it.
#[derive(Clone, Debug, PartialEq)]
struct Binding {
    /// `prefix %`, or a bare `M-f` for a root-table key.
    key: String,
    /// The arguments the binding passes (`-h`), or `""`.
    args: String,
}

fn meta_of(item: &Item) -> Option<&CommandMeta> {
    item.data.as_ref()?.downcast_ref::<CommandMeta>()
}

/// The usage string stashed on a command item, or `""` for a row that carries
/// none (the synthetic "Run:" row, or a help row).
fn usage_of(item: &Item) -> &str {
    meta_of(item).map_or("", |m| m.usage.as_str())
}

/// The first word of `s` and what follows it, both trimmed at the front.
fn split_word(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    let end = s.find(char::is_whitespace).unwrap_or(s.len());
    (&s[..end], s[end..].trim_start())
}

/// The keys bound to each command, from `tmux list-keys` output, keyed by the
/// command's full name (`names` maps names and aliases to it).
///
/// Only the prefix and root tables count: the copy-mode tables bind
/// `send-keys -X` actions, not commands. A key that runs the command comes
/// first; one that prompts for it (`command-prompt -I "#W" { rename-window
/// "%%" }`, `confirm-before ... kill-window`) follows, since pressing it still
/// gets you there.
fn parse_bindings(
    list_keys: &str,
    names: &HashMap<String, String>,
) -> HashMap<String, Vec<Binding>> {
    let mut direct: HashMap<String, Vec<Binding>> = HashMap::new();
    let mut prompted: HashMap<String, Vec<Binding>> = HashMap::new();
    for line in list_keys.lines() {
        let Some(mut rest) = line.trim().strip_prefix("bind-key") else {
            continue;
        };
        let mut table = "prefix";
        loop {
            match split_word(rest) {
                ("-r", after) => rest = after,
                ("-T", after) => (table, rest) = split_word(after),
                _ => break,
            }
        }
        let (key, command) = split_word(rest);
        // list-keys escapes keys that mean something to the parser: \" \% \;
        let key = key
            .strip_prefix('\\')
            .filter(|k| !k.is_empty())
            .unwrap_or(key);
        let key = match table {
            "prefix" => format!("prefix {}", key),
            "root" => key.to_string(),
            _ => continue,
        };
        let (first, args) = split_word(command);
        if let Some(name) = names.get(first) {
            direct.entry(name.clone()).or_default().push(Binding {
                key,
                args: args.to_string(),
            });
        } else if first == "command-prompt" || first == "confirm-before" {
            let target = args
                .split_whitespace()
                .map(|w| w.trim_matches(|c| c == '{' || c == '}'))
                .find_map(|w| names.get(w));
            if let Some(name) = target {
                prompted.entry(name.clone()).or_default().push(Binding {
                    key,
                    args: String::new(),
                });
            }
        }
    }
    for (name, keys) in prompted {
        direct.entry(name).or_default().extend(keys);
    }
    direct
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
    let has_required = !parse_usage(name, usage).iter().all(|p| p.optional);
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
            name: name.to_string(),
            usage: usage.to_string(),
            bindings: Vec::new(),
        })),
        ..Default::default()
    })
}

/// Attach each command's key bindings to its row.
fn with_bindings(mut items: Vec<Item>, list_keys: &str) -> Vec<Item> {
    let mut names = HashMap::new();
    for it in &items {
        names.insert(it.title.clone(), it.title.clone());
        for a in it.aliases.iter().flatten() {
            names.insert(a.clone(), it.title.clone());
        }
    }
    let mut bindings = parse_bindings(list_keys, &names);
    for it in &mut items {
        let Some(meta) = meta_of(it) else {
            continue;
        };
        let meta = CommandMeta {
            name: meta.name.clone(),
            usage: meta.usage.clone(),
            bindings: bindings.remove(&it.title).unwrap_or_default(),
        };
        it.data = Some(Rc::new(meta));
    }
    items
}

/// Every tmux command, straight from `tmux list-commands`, ordered by topic so
/// the grouped resting list reads top to bottom without repeating headers.
fn build_items() -> Vec<Item> {
    let items: Vec<Item> = tmux(&["list-commands"])
        .lines()
        .filter_map(parse_command_line)
        .collect();
    let mut items = with_bindings(items, &tmux(&["list-keys"]));
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

/// The command the input has committed to: its first word names a command (or
/// alias) and a space follows it. Until that space the word may still be the
/// start of a longer name — `rename` is `rename-session`'s alias, but you may
/// be on your way to `rename-window` — so it only narrows the list.
///
/// Returns the command and the byte length of the name as typed.
fn committed_command<'a>(items: &'a [Item], input: &str) -> Option<(&'a Item, usize)> {
    let end = input.find(char::is_whitespace)?;
    let head = &input[..end];
    items
        .iter()
        .find(|it| command_named(it, head))
        .map(|cmd| (cmd, end))
}

/// The committed command name, drawn as a pill and deleted as one unit.
fn command_chip(items: &[Item], input: &str) -> Option<usize> {
    committed_command(items, input).map(|(_, end)| input[..end].chars().count())
}

/// Filter: always offer a "Run: <query>" row on top. Once the input commits to
/// a command (see `committed_command`), show that command — description and
/// all — and expand its arguments as help rows, which stay put while you type
/// them. Otherwise fuzzy-rank the command list by name, so a partial word just
/// narrows the choices without prematurely committing to one.
///
/// `query` arrives with its trailing whitespace, which is what commits.
fn filter_commands(items: &[Item], query: &str) -> Vec<Item> {
    let mut out = Vec::new();
    out.push(run_typed_item(query.trim()));

    if let Some((cmd, end)) = committed_command(items, query) {
        // The Run row carries the command along, so the details strip can
        // describe it while the Run row is highlighted.
        out[0].data = cmd.data.clone();
        let typed = typed_flags(&query[end..]);
        let mut params = parse_usage(&cmd.title, usage_of(cmd));
        for p in &mut params {
            p.used = p.flag.is_some_and(|f| typed.contains(&f));
        }
        // A heading now, not a choice: Enter belongs to the Run row, and Tab
        // must not swap the arguments you've typed for the bare name.
        out.push(Item {
            selectable: Some(false),
            complete: None,
            ..cmd.clone()
        });
        out.extend(params.iter().map(help_item));
        return out;
    }

    out.extend(match_commands(items, query.trim()));
    out
}

// ---- rendering ----------------------------------------------------------------

/// Cells before a help row's tick mark, lining the params up under the
/// command's name.
const HELP_INDENT: i64 = 5;
/// Cells the `optional` / `required` column takes, gap included.
const HELP_TAG_W: i64 = 10;

fn pad(s: &str, width: i64) -> String {
    let gap = (width - display_width(s)).max(0) as usize;
    format!("{}{}", s, " ".repeat(gap))
}

/// Render a help row: `<indent><param>  <optional|required>  <description>`,
/// dim, with required args flagged in the accent color and flags you've
/// already typed ticked off. Emits only foreground color switches so the row
/// background (set by the list composer) is preserved.
fn render_help_param(p: &HelpParam, colors: &Colors, width: i64) -> String {
    let (tag, tag_color) = if p.optional {
        ("optional", &colors.muted)
    } else {
        ("required", &colors.accent)
    };
    let (mark, display_color) = if p.used {
        (format!("{}✓ ", colors.accent), &colors.fg)
    } else {
        ("  ".to_string(), &colors.muted)
    };
    let col = p.col.max(display_width(&p.display) + 1);
    let budget = width - HELP_INDENT - 2 - col - HELP_TAG_W;
    let desc = p.desc.as_deref().unwrap_or_default();
    format!(
        "{}{}{}{}{}{}{}{}",
        " ".repeat(HELP_INDENT as usize),
        mark,
        display_color,
        pad(&p.display, col),
        tag_color,
        pad(tag, HELP_TAG_W),
        colors.muted,
        if budget > 0 {
            clip(desc, budget)
        } else {
            String::new()
        },
    )
}

// ---- details strip ------------------------------------------------------------

/// What the strip says on the Run row before a command is committed.
const RUN_HINT: &str = "Runs the line exactly as typed, like tmux's own prompt. \
     Highlight a command to read about it here; Tab completes it.";
/// Key bindings shown on the strip's rule before the rest are counted.
const MAX_BINDINGS: usize = 3;
/// Binding arguments longer than this are left off the rule.
const MAX_BINDING_ARGS: i64 = 12;

/// Greedy word wrap into at most `max_lines` lines of `width` cells; the last
/// line is clipped with an ellipsis when the text runs over.
fn wrap(text: &str, width: i64, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut words = text.split_whitespace();
    while let Some(word) = words.next() {
        let fits = lines
            .last()
            .is_some_and(|l| display_width(l) + 1 + display_width(word) <= width);
        if fits {
            let last = lines.last_mut().expect("checked above");
            last.push(' ');
            last.push_str(word);
        } else if lines.len() < max_lines {
            lines.push(word.to_string());
        } else {
            let rest: Vec<&str> = std::iter::once(word).chain(words.by_ref()).collect();
            let last = lines.last_mut().expect("max_lines > 0");
            *last = clip(&format!("{} {}", last, rest.join(" ")), width);
            break;
        }
    }
    lines
}

/// The strip's top rule, carrying the keys bound to the command:
/// `── [prefix "]  [prefix %] -h  ────────`.
fn rule_line(bindings: &[Binding], width: i64, colors: &Colors) -> String {
    let mut out = format!("{}──", colors.muted);
    let mut used = 2;
    let mut shown = 0;
    for b in bindings.iter().take(MAX_BINDINGS) {
        let args = if display_width(&b.args) <= MAX_BINDING_ARGS {
            b.args.as_str()
        } else {
            ""
        };
        let cost = 2
            + display_width(&b.key)
            + 2
            + if args.is_empty() {
                0
            } else {
                1 + display_width(args)
            };
        if used + cost + 2 > width {
            break;
        }
        out.push_str(&format!(
            "  {}{} {} {}{}{}",
            colors.bg, colors.fg, b.key, colors.reset, colors.panel, colors.muted
        ));
        if !args.is_empty() {
            out.push_str(&format!(" {}", args));
        }
        used += cost;
        shown += 1;
    }
    let more = bindings.len() - shown;
    if more > 0 && used + 10 < width {
        let note = format!("  +{} more", more);
        used += display_width(&note);
        out.push_str(&note);
    }
    if used > 2 {
        out.push_str("  ");
        used += 2;
    }
    out.push_str(&"─".repeat((width - used).max(0) as usize));
    out.push_str(&format!("{}{}", colors.reset, colors.panel));
    out
}

/// The strip under the list: the focused command's key bindings on a rule,
/// what it does, and an example. The focused command is the highlighted row's
/// — or, on the Run row, the one the input has committed to.
fn render_details(item: Option<&Item>, ctx: &PreviewCtx) -> Vec<String> {
    let c = ctx.colors;
    let end = format!("{}{}", c.reset, c.panel);
    let width = ctx.width;
    let meta = item.and_then(meta_of);
    let help = meta.and_then(|m| help_for(&m.name));

    let about = match (meta, help) {
        (_, Some(h)) => h.about.to_string(),
        (Some(m), None) => desc_for(&m.name)
            .map(str::to_string)
            .unwrap_or_else(|| m.usage.clone()),
        (None, _) => RUN_HINT.to_string(),
    };
    let bindings = meta.map_or(&[][..], |m| m.bindings.as_slice());

    let mut lines = vec![rule_line(bindings, width, c)];
    let mut about_lines = wrap(&about, width, 2);
    about_lines.resize(2, String::new());
    lines.extend(
        about_lines
            .into_iter()
            .map(|l| format!("{}{}{}", c.fg, l, end)),
    );
    if let Some(h) = help.filter(|h| !h.example.is_empty()) {
        lines.push(format!(
            "{}e.g. {}{}{}",
            c.muted,
            c.accent,
            clip(h.example, width - 5),
            end
        ));
    }
    lines.truncate(ctx.height.max(0) as usize);
    lines
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
        chip: Some(Rc::new(command_chip)),
        details: Some(Rc::new(render_details)),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::super::command_help::HELP;
    use super::*;

    fn params(usage: &str) -> Vec<(String, bool, Option<String>)> {
        parse_usage("", usage)
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
    fn committed_command_expands_help_rows() {
        let items = vec![parse_command_line("rename-window [-t target-window] new-name").unwrap()];
        let vis = filter_commands(&items, "rename-window ");
        assert_eq!(vis[0].title, "Run: rename-window");
        assert_eq!(vis[1].title, "rename-window");
        // The command keeps its description while you type parameters; the usage
        // it was parsed from rides in `data`, not on the row.
        assert_eq!(vis[1].description.as_deref(), Some("rename a window"));
        assert_eq!(usage_of(&vis[1]), "[-t target-window] new-name");
        // It is a heading now: Enter runs the Run row, and Tab has nothing to
        // swap the typed arguments for.
        assert_eq!(vis[1].selectable, Some(false));
        assert_eq!(vis[1].complete, None);
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
    fn a_name_commits_once_a_space_follows_it() {
        let items = vec![
            parse_command_line("rename-window (renamew) [-t target-window] new-name").unwrap(),
        ];
        // Partial name: no help rows, just the ranked command under the Run row.
        assert!(!has_help_rows(&filter_commands(&items, "rename-w")));
        // The full name, still being typed: a list, not yet a commitment.
        assert!(!has_help_rows(&filter_commands(&items, "rename-window")));
        // A space commits: help expands.
        assert!(has_help_rows(&filter_commands(&items, "rename-window ")));
        // Exact name plus a typed parameter: help stays put.
        let typing = filter_commands(&items, "rename-window my-name");
        assert_eq!(typing[0].title, "Run: rename-window my-name");
        assert!(has_help_rows(&typing));
        // The alias triggers it the same way.
        assert!(has_help_rows(&filter_commands(&items, "renamew foo")));
    }

    /// `rename` is `rename-session`'s alias, and `new` is `new-session`'s — but
    /// typing either must not hide the `-window` command you may be heading for.
    #[test]
    fn an_alias_does_not_hide_its_siblings_until_committed() {
        let items = vec![
            parse_command_line("rename-session (rename) [-t target-session] new-name").unwrap(),
            parse_command_line("rename-window (renamew) [-t target-window] new-name").unwrap(),
        ];
        let titles = |q: &str| -> Vec<String> {
            filter_commands(&items, q)
                .into_iter()
                .map(|i| i.title)
                .collect()
        };
        assert_eq!(
            titles("rename"),
            vec!["Run: rename", "rename-session", "rename-window"]
        );
        // Once a space commits to the alias, it means rename-session.
        let committed = titles("rename ");
        assert_eq!(committed[1], "rename-session");
        assert!(!committed.contains(&"rename-window".to_string()));
    }

    #[test]
    fn the_pill_is_the_committed_command_name() {
        let items = vec![parse_command_line("split-window (splitw) [-bdfhIvPZ]").unwrap()];
        assert_eq!(command_chip(&items, "split-window -h"), Some(12));
        assert_eq!(command_chip(&items, "splitw "), Some(6));
        // Not yet committed, not a command, or not at the start: no pill.
        assert_eq!(command_chip(&items, "split-window"), None);
        assert_eq!(command_chip(&items, "split-pane "), None);
        assert_eq!(command_chip(&items, " split-window "), None);
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

    /// `tmux list-commands` from tmux 3.4, and the same list as tmux's
    /// development version (3.9-next, October 2026) would print it, built from
    /// its source.
    const LIST_COMMANDS: &[&str] = &[
        include_str!("testdata/tmux-3.4.txt"),
        include_str!("testdata/tmux-next-3.9.txt"),
    ];

    /// Every command either version has, with each usage it was seen with.
    fn all_usages() -> Vec<(String, String)> {
        let mut out = Vec::new();
        for list in LIST_COMMANDS {
            for item in list.lines().filter_map(parse_command_line) {
                out.push((item.title.clone(), usage_of(&item).to_string()));
            }
        }
        out
    }

    fn all_commands() -> Vec<String> {
        let mut names: Vec<String> = all_usages().into_iter().map(|(n, _)| n).collect();
        names.sort();
        names.dedup();
        names
    }

    #[test]
    fn every_tmux_command_has_a_real_icon() {
        for name in &all_commands() {
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
        // ...and, once committed, expands into help rows.
        let vis = filter_commands(&items, "rename-window ");
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
        let all = all_commands();
        for name in &names {
            assert!(
                all.iter().any(|n| n == name),
                "{name} is not a tmux command"
            );
        }
    }

    #[test]
    fn every_tmux_command_has_a_description() {
        for name in &all_commands() {
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

    // ---- argument help ---------------------------------------------------------

    fn displays(params: &[HelpParam]) -> Vec<&str> {
        params.iter().map(|p| p.display.as_str()).collect()
    }

    #[test]
    fn a_letter_listed_twice_keeps_only_its_valued_form() {
        // tmux 3.4's split-window lists `e` as a bare flag and as `-e environment`.
        let got = parse_usage("", "[-bdefhIPvZ] [-e environment] [-t target-pane]");
        let d = displays(&got);
        assert!(d.contains(&"-e environment"));
        assert!(!d.contains(&"-e"));
        // Nor does the cluster of undescribed flags it would otherwise join.
        let bare: Vec<&&str> = d.iter().filter(|x| !x.contains(' ')).collect();
        assert!(bare.iter().all(|x| !x.contains('e')), "{bare:?}");
    }

    #[test]
    fn a_repeatable_argument_is_one_row() {
        let got = parse_usage("", "[-t target-pane] [shell-command [argument ...]]");
        assert_eq!(
            displays(&got),
            vec!["-t target-pane", "shell-command", "argument ..."]
        );
    }

    #[test]
    fn alternative_flags_are_separate_rows() {
        let got = parse_usage("wait-for", "[-L|-S|-U] channel");
        assert_eq!(displays(&got), vec!["-L", "-S", "-U", "channel"]);
        assert!(got.iter().all(|p| p.desc.is_some()));
    }

    #[test]
    fn params_share_one_column_wide_enough_for_the_longest() {
        let got = parse_usage("", "[-d] [-c start-directory] [-t target-pane]");
        let col = display_width("-c start-directory") + 2;
        assert!(got.iter().all(|p| p.col == col));
    }

    #[test]
    fn typed_flags_reads_clusters_and_stops_at_double_dash() {
        assert_eq!(typed_flags(" -h -l 30% -dZ"), vec!['h', 'l', 'd', 'Z']);
        assert_eq!(typed_flags(" -t work -- -x"), vec!['t']);
        assert!(typed_flags(" logs").is_empty());
    }

    #[test]
    fn typed_flags_are_ticked_in_the_help_rows() {
        let items = vec![parse_command_line(
            "split-window (splitw) [-bdfhIvPZ] [-l size] [-t target-pane] [shell-command]",
        )
        .unwrap()];
        let vis = filter_commands(&items, "splitw -d -l 30%");
        let used: Vec<&str> = vis
            .iter()
            .filter_map(|i| i.data.as_ref()?.downcast_ref::<HelpParam>())
            .filter(|p| p.used)
            .map(|p| p.display.as_str())
            .collect();
        assert_eq!(used, vec!["-d", "-l size"]);
    }

    #[test]
    fn the_run_row_carries_the_committed_command_for_the_details_strip() {
        let items = vec![parse_command_line("kill-pane (killp) [-a] [-t target-pane]").unwrap()];
        let vis = filter_commands(&items, "killp -a");
        assert_eq!(meta_of(&vis[0]).map(|m| m.name.as_str()), Some("kill-pane"));
        // Free text commits to nothing, so the strip shows its hint instead.
        let vis = filter_commands(&items, "kil");
        assert!(meta_of(&vis[0]).is_none());
    }

    // ---- bundled help ----------------------------------------------------------

    #[test]
    fn help_table_is_sorted_and_names_real_commands() {
        let names: Vec<&str> = HELP.iter().map(|h| h.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            names, sorted,
            "HELP must be sorted by name, without repeats"
        );
        let all = all_commands();
        for name in &names {
            assert!(
                all.iter().any(|n| n == name),
                "{name} is not a tmux command"
            );
        }
    }

    /// Every flag either tmux version prints gets its own described row — none
    /// collapse into the "flags" catch-all, the gap that started this table.
    #[test]
    fn every_command_has_help_and_every_flag_a_description() {
        for (name, usage) in all_usages() {
            let help = help_for(&name).unwrap_or_else(|| panic!("{name} has no help"));
            for p in parse_usage(&name, &usage) {
                if let Some(f) = p.flag {
                    assert!(help.flag(f).is_some(), "{name} -{f} has no description");
                }
                assert_ne!(
                    p.desc.as_deref(),
                    Some("flags"),
                    "{name}: {} is undescribed",
                    p.display
                );
            }
        }
    }

    /// Nothing describes a flag that neither version has.
    #[test]
    fn help_flags_are_real_flags() {
        let usages = all_usages();
        for h in HELP {
            for (letter, _) in h.flags {
                let known = usages
                    .iter()
                    .filter(|(n, _)| n == h.name)
                    .any(|(n, u)| parse_usage(n, u).iter().any(|p| p.flag == Some(*letter)));
                assert!(known, "{} has no -{letter}", h.name);
            }
        }
    }

    /// Flag and argument text must fit beside the widest param at the default
    /// popup width; the about text, two lines of the strip; the example, one.
    #[test]
    fn help_text_fits_where_it_is_shown() {
        for h in HELP {
            for (letter, d) in h.flags {
                assert!(display_width(d) <= 46, "{} -{letter}: {d}", h.name);
            }
            for (arg, d) in h.args {
                assert!(display_width(d) <= 46, "{} {arg}: {d}", h.name);
            }
            assert!(display_width(h.about) <= 170, "{}: about", h.name);
            assert!(display_width(h.example) <= 60, "{}: example", h.name);
        }
    }

    /// The flags an example passes, read the way tmux's getopt reads them: up
    /// to the first argument that isn't a flag or a flag's value.
    fn example_flags(example_args: &str, params: &[HelpParam]) -> Vec<char> {
        let valued: Vec<char> = params
            .iter()
            .filter(|p| p.display.starts_with('-') && p.display.contains(' '))
            .filter_map(|p| p.flag)
            .collect();
        let mut out = Vec::new();
        let mut toks = example_args.split_whitespace();
        while let Some(tok) = toks.next() {
            let Some(letters) = tok.strip_prefix('-').filter(|l| !l.is_empty()) else {
                break;
            };
            out.extend(letters.chars());
            if letters.chars().last().is_some_and(|c| valued.contains(&c)) {
                toks.next();
            }
        }
        out
    }

    /// Each example runs its own command, using only flags tmux 3.4 has (or the
    /// development version, for a command 3.4 lacks).
    #[test]
    fn examples_use_real_flags() {
        let old: Vec<Item> = LIST_COMMANDS[0]
            .lines()
            .filter_map(parse_command_line)
            .collect();
        let new: Vec<Item> = LIST_COMMANDS[1]
            .lines()
            .filter_map(parse_command_line)
            .collect();
        for h in HELP {
            let cmd = old
                .iter()
                .chain(&new)
                .find(|i| i.title == h.name)
                .expect("checked by help_table_is_sorted_and_names_real_commands");
            let (head, args) = split_word(h.example);
            assert!(command_named(cmd, head), "{}: example runs {head}", h.name);
            let params = parse_usage(h.name, usage_of(cmd));
            for f in example_flags(args, &params) {
                assert!(
                    params.iter().any(|p| p.flag == Some(f)),
                    "{}: example passes -{f}, which its usage lacks",
                    h.name
                );
            }
        }
    }

    // ---- key bindings ----------------------------------------------------------

    fn names() -> HashMap<String, String> {
        [
            ("split-window", "split-window"),
            ("splitw", "split-window"),
            ("rename-window", "rename-window"),
            ("kill-window", "kill-window"),
            ("send-keys", "send-keys"),
            ("run-shell", "run-shell"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
    }

    const LIST_KEYS: &str = r##"bind-key    -T copy-mode    C-r                    send-keys -X search-backward
bind-key    -T prefix       "                     split-window
bind-key    -T prefix       \%                     split-window -h
bind-key    -T prefix       &                      confirm-before -p "kill-window #W? (y/n)" kill-window
bind-key    -T prefix       ,                      command-prompt -I "#W" { rename-window "%%" }
bind-key -r -T prefix       M-v                    splitw -v -l 30%
bind-key    -T root         C-Space                run-shell /opt/tmux-palette
"##;

    #[test]
    fn bindings_come_from_the_prefix_and_root_tables() {
        let b = parse_bindings(LIST_KEYS, &names());
        let keys = |name: &str| -> Vec<(String, String)> {
            b.get(name)
                .map(|v| v.iter().map(|x| (x.key.clone(), x.args.clone())).collect())
                .unwrap_or_default()
        };
        assert_eq!(
            keys("split-window"),
            vec![
                ("prefix \"".to_string(), String::new()),
                ("prefix %".to_string(), "-h".to_string()),
                ("prefix M-v".to_string(), "-v -l 30%".to_string()),
            ]
        );
        // Through a prompt: the key still gets you to the command.
        assert_eq!(
            keys("rename-window"),
            vec![("prefix ,".to_string(), String::new())]
        );
        assert_eq!(
            keys("kill-window"),
            vec![("prefix &".to_string(), String::new())]
        );
        // Root-table keys show bare; copy-mode tables don't count.
        assert_eq!(keys("run-shell")[0].0, "C-Space");
        assert!(keys("send-keys").is_empty());
    }

    // ---- details strip ---------------------------------------------------------

    #[test]
    fn wrap_fills_lines_and_clips_the_last() {
        assert_eq!(wrap("one two three", 7, 2), vec!["one two", "three"]);
        let got = wrap("one two three four five", 9, 2);
        assert_eq!(got[0], "one two");
        assert!(got[1].ends_with('…') && display_width(&got[1]) <= 9);
    }

    #[test]
    fn the_rule_shows_keys_then_counts_the_rest() {
        let colors = Colors::default();
        let b = |k: &str| Binding {
            key: k.to_string(),
            args: String::new(),
        };
        let bindings: Vec<Binding> = (0..5).map(|i| b(&format!("prefix {}", i))).collect();
        let line = crate::text::strip(&rule_line(&bindings, 80, &colors));
        assert!(line.contains(" prefix 0 ") && line.contains(" prefix 2 "));
        assert!(!line.contains("prefix 3"));
        assert!(line.contains("+2 more"));
        assert_eq!(display_width(&line), 80);
    }

    /// `join-pane` and `move-pane` are the same command in tmux — identical
    /// usage, and joining within one window works for both.
    #[test]
    fn join_and_move_pane_read_alike() {
        assert_eq!(desc_for("join-pane"), desc_for("move-pane"));
    }
}
