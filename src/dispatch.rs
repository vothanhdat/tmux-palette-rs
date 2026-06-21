//! Action encoding for the launcher — port of `src/dispatch.ts`.
//!
//! When an item is activated, the palette writes the encoded command to a
//! tempfile and exits; the launcher reads it *after* the popup closes and runs
//! it. Two prefixes: `tmux:<cmd>` runs `tmux <cmd>`, `shell:<cmd>` runs it
//! directly. `palette` is sugar for re-launching another palette.

use std::fs;

use crate::theme::{popup_flags, resolve_active_theme};
use crate::types::Action;

fn encode_action(action: &Action) -> Option<String> {
    match action {
        Action::Tmux(cmd) => Some(format!("tmux:{}", cmd)),
        Action::Shell(cmd) => Some(format!("shell:{}", cmd)),
        Action::Popup(p) => Some(format!(
            "tmux:display-popup -E {} -h 80% -w 80% {}",
            popup_flags(&resolve_active_theme(&None), None),
            p.popup
        )),
        Action::Palette(name) => {
            let bin =
                std::env::var("TMUX_PALETTE_BIN").unwrap_or_else(|_| "tmux-palette".to_string());
            Some(format!("tmux:run-shell -b '{} {}'", bin, name))
        }
        Action::Run(_) | Action::Apply(_) => None,
    }
}

pub fn dispatch_to_file(action: &Action, cmd_file: Option<&str>) {
    if let (Some(encoded), Some(file)) = (encode_action(action), cmd_file) {
        let _ = fs::write(file, encoded);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PopupAction;
    use std::sync::Mutex;

    // Serializes tests that mutate process-global env vars.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn dispatch_line(action: &Action) -> String {
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let file =
            std::env::temp_dir().join(format!("tmux-palette-test-{}-{}", std::process::id(), n));
        dispatch_to_file(action, file.to_str());
        let out = std::fs::read_to_string(&file).unwrap();
        let _ = std::fs::remove_file(&file);
        out
    }

    #[test]
    fn encodes_tmux_commands() {
        assert_eq!(
            dispatch_line(&Action::Tmux("split-window -h".into())),
            "tmux:split-window -h"
        );
    }

    #[test]
    fn encodes_shell_commands() {
        assert_eq!(
            dispatch_line(&Action::Shell("echo hi".into())),
            "shell:echo hi"
        );
    }

    #[test]
    fn encodes_palette_actions_with_configured_launcher() {
        let _guard = ENV_LOCK.lock().unwrap();
        let prev = std::env::var("TMUX_PALETTE_BIN").ok();
        std::env::set_var("TMUX_PALETTE_BIN", "/tmp/tmux-palette");
        let got = dispatch_line(&Action::Palette("themes".into()));
        match prev {
            Some(v) => std::env::set_var("TMUX_PALETTE_BIN", v),
            None => std::env::remove_var("TMUX_PALETTE_BIN"),
        }
        assert_eq!(got, "tmux:run-shell -b '/tmp/tmux-palette themes'");
    }

    #[test]
    fn encodes_popup_actions_with_default_sizing() {
        let line = dispatch_line(&Action::Popup(PopupAction {
            popup: "htop".into(),
            ..Default::default()
        }));
        assert!(line.starts_with("tmux:display-popup -E -B -s '"));
        assert!(line.ends_with(" -h 80% -w 80% htop"));
    }
}
