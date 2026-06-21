//! tmux-palette binary entry point.
//!
//! A single self-contained binary with three modes (no bash wrapper needed):
//!   * **launcher** (default): sizes and opens a `tmux display-popup` running
//!     itself, then runs the chosen command *after* the popup closes — the trick
//!     that lets interactive tmux prompts (`confirm-before`, `command-prompt`)
//!     receive stdin.
//!   * **run** (when `TMUX_PALETTE_CMD` is set, i.e. inside the popup): the
//!     interactive TUI.
//!   * **measure** (`--measure`): print popup geometry (kept for parity/testing).

use std::fs;
use std::process::Command;

use tmux_palette::cli::{apply_category, load_palette, make_loader, measure};
use tmux_palette::palette::run_palette;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let name = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "commands".to_string());
    let category = args
        .iter()
        .find_map(|a| a.strip_prefix("--category="))
        .map(|s| s.to_string());

    if args.iter().any(|a| a == "--measure") {
        measure_mode(&name, category.as_deref(), &args);
    } else if std::env::var_os("TMUX_PALETTE_CMD").is_some() {
        run_mode(&name, category.as_deref());
    } else {
        launcher_mode(&name, category.as_deref(), &args);
    }
}

fn run_mode(name: &str, category: Option<&str>) {
    let Some(mut def) = load_palette(name) else {
        eprintln!(
            "Unknown palette: {}. Built-in: commands, find-pane, move-pane, themes. \
             Custom palettes go in ~/.config/tmux-palette/palettes/<name>.json",
            name
        );
        std::process::exit(1);
    };
    if let Some(cat) = category.filter(|c| !c.is_empty()) {
        def = apply_category(def, cat);
    }
    run_palette(def, Some(make_loader()), name);
}

fn arg_num(args: &[String], prefix: &str) -> Option<i64> {
    args.iter()
        .find_map(|a| a.strip_prefix(prefix))
        .and_then(|s| s.parse::<i64>().ok())
}

fn measure_mode(name: &str, category: Option<&str>, args: &[String]) {
    let cw = arg_num(args, "--cw=").unwrap_or(0);
    let ch = arg_num(args, "--ch=").unwrap_or(0);
    let Some(mut def) = load_palette(name) else {
        std::process::exit(1);
    };
    if let Some(cat) = category.filter(|c| !c.is_empty()) {
        def = apply_category(def, cat);
    }
    let m = measure(&def, cw, ch);
    println!(
        "{}\t{}\t{}\t{}\t{}\t{}",
        m.rows, m.width, m.pad_x, m.border, m.body_style, m.border_style
    );
}

/// Single-quote a value for safe embedding in the popup shell command.
fn shq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn tmux_num(fmt: &str) -> Option<i64> {
    let out = Command::new("tmux")
        .args(["display-message", "-p", fmt])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<i64>()
        .ok()
}

fn launcher_mode(name: &str, category: Option<&str>, args: &[String]) {
    if std::env::var_os("TMUX").is_none() {
        eprintln!("tmux-palette: must be run inside a tmux session");
        std::process::exit(1);
    }

    let self_path = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "tmux-palette".to_string());

    let ch = tmux_num("#{client_height}").unwrap_or(24);
    let cw = tmux_num("#{client_width}").unwrap_or(80);

    // Ask the palette how big it wants to be (defaults + sizing.json applied).
    let mut def = match load_palette(name) {
        Some(d) => d,
        None => {
            eprintln!("tmux-palette: unknown palette '{}'", name);
            std::process::exit(1);
        }
    };
    if let Some(cat) = category.filter(|c| !c.is_empty()) {
        def = apply_category(def, cat);
    }
    let m = measure(&def, cw, ch);

    // Cap by client size, leaving breathing room (mobile mode uses full dims).
    let max_h = ch - 2;
    let mut h = if m.rows > max_h { max_h } else { m.rows };
    let mut w = if m.width > cw - 4 { cw - 4 } else { m.width };
    if m.width >= cw {
        h = ch;
        w = cw;
    }
    if let Some(v) = std::env::var("TMUX_PALETTE_HEIGHT")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
    {
        h = v;
    }
    if let Some(v) = std::env::var("TMUX_PALETTE_WIDTH")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
    {
        w = v;
    }

    let bordered = m.border != "none";

    // Temp file the palette writes the chosen command into.
    let cmd_file = std::env::temp_dir().join(format!(
        "tmux-palette-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = fs::write(&cmd_file, b"");
    let cmd_file = cmd_file.to_string_lossy().into_owned();

    // Build the inner command: env vars + exec self with the forwarded args.
    let forwarded: String = args.iter().map(|a| shq(a)).collect::<Vec<_>>().join(" ");
    let inner = format!(
        "TMUX_PALETTE_CMD={} TMUX_PALETTE_BIN={} TMUX_PALETTE_PADX={} TMUX_PALETTE_BORDERED={} exec {} {}",
        shq(&cmd_file),
        shq(&self_path),
        m.pad_x,
        if bordered { 1 } else { 0 },
        shq(&self_path),
        forwarded
    );

    let mut popup = Command::new("tmux");
    popup.arg("display-popup");
    if bordered {
        popup.args(["-b", &m.border, "-s", &m.body_style, "-S", &m.border_style]);
    } else {
        popup.args(["-B", "-s", &m.body_style]);
    }
    popup.args(["-w", &w.to_string(), "-h", &h.to_string(), "-E", &inner]);
    let _ = popup.status();

    // After the popup closes, run the dispatched command.
    let content = fs::read_to_string(&cmd_file).unwrap_or_default();
    let _ = fs::remove_file(&cmd_file);
    if let Some(rest) = content.strip_prefix("tmux:") {
        // Through a shell so `\;` separators and quoting are interpreted, like
        // the original `eval "tmux ..."`. Exit status is intentionally ignored.
        let _ = Command::new("sh")
            .arg("-c")
            .arg(format!("tmux {}", rest))
            .status();
    } else if let Some(rest) = content.strip_prefix("shell:") {
        let _ = Command::new("sh").arg("-c").arg(rest).status();
    }
}
