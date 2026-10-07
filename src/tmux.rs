//! Thin tmux command helpers — port of `src/tmux.ts`.

use std::process::Command;

/// Run `tmux <args>` and return trimmed stdout (empty string on failure).
pub fn tmux(args: &[&str]) -> String {
    tmux_raw(args).trim_end().to_string()
}

/// Run `tmux <args>` and return stdout as-is (empty string on failure) — for
/// output whose trailing whitespace is content, like a captured screen whose
/// last row is a colored bar.
pub fn tmux_raw(args: &[&str]) -> String {
    match Command::new("tmux").args(args).output() {
        Ok(out) => String::from_utf8_lossy(&out.stdout).into_owned(),
        Err(_) => String::new(),
    }
}

/// Single-quote a value for safe embedding in a tmux/shell command.
pub fn tmux_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_embedded_single_quotes() {
        assert_eq!(tmux_quote("a'b"), "'a'\\''b'");
        assert_eq!(tmux_quote("plain"), "'plain'");
    }
}
