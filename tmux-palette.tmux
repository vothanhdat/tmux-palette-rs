#!/usr/bin/env bash
# TPM entry point for tmux-palette (Rust port).
# Sourced by tmux when the plugin is installed via tmux-plugins/tpm.

set -eu

CURRENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$CURRENT_DIR/target/release/tmux-palette"

# Build the release binary on first load if it isn't there yet.
if [ ! -x "$BIN" ]; then
  if ! command -v cargo >/dev/null 2>&1; then
    tmux display-message "tmux-palette: cargo not found. Install Rust: https://rustup.rs"
    exit 0
  fi
  (cd "$CURRENT_DIR" && cargo build --release) >/dev/null 2>&1 || {
    tmux display-message "tmux-palette: 'cargo build --release' failed in $CURRENT_DIR"
    exit 0
  }
fi

get_opt() {
  local val
  val="$(tmux show-option -gqv "$1" 2>/dev/null || true)"
  echo "${val:-$2}"
}

PALETTE_KEY="$(get_opt @palette-key 'C-Space')"
FIND_PANE_KEY="$(get_opt @palette-find-pane-key '')"
MOVE_PANE_KEY="$(get_opt @palette-move-pane-key '')"

if [ "$PALETTE_KEY" != "off" ] && [ -n "$PALETTE_KEY" ]; then
  tmux bind-key -n "$PALETTE_KEY" run-shell "$BIN"
fi

if [ -n "$FIND_PANE_KEY" ]; then
  tmux bind-key -n "$FIND_PANE_KEY" run-shell "$BIN find-pane"
fi

if [ -n "$MOVE_PANE_KEY" ]; then
  tmux bind-key -n "$MOVE_PANE_KEY" run-shell "$BIN move-pane"
fi
