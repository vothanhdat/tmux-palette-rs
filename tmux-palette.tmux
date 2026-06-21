#!/usr/bin/env bash
# TPM entry point for tmux-palette (Rust port).
# Sourced by tmux when the plugin is installed via tmux-plugins/tpm.
#
# Prefers a prebuilt binary shipped in dist/ (so installing from the `stable`
# branch needs no Rust toolchain); falls back to `cargo build --release`.

set -eu

CURRENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Map the current platform to a prebuilt binary name, if one exists.
prebuilt_name() {
  case "$(uname -s)/$(uname -m)" in
    Linux/x86_64)        echo "tmux-palette-linux-x64" ;;
    Darwin/arm64)        echo "tmux-palette-macos-arm64" ;;
    *)                   echo "" ;;
  esac
}

BIN=""
PREBUILT="$(prebuilt_name)"
if [ -n "$PREBUILT" ] && [ -f "$CURRENT_DIR/dist/$PREBUILT" ]; then
  BIN="$CURRENT_DIR/dist/$PREBUILT"
  chmod +x "$BIN" 2>/dev/null || true
elif [ -x "$CURRENT_DIR/target/release/tmux-palette" ]; then
  BIN="$CURRENT_DIR/target/release/tmux-palette"
fi

# No usable binary yet — build one if cargo is available.
if [ -z "$BIN" ]; then
  if ! command -v cargo >/dev/null 2>&1; then
    tmux display-message "tmux-palette: no prebuilt binary for this platform and cargo not found. Install Rust: https://rustup.rs"
    exit 0
  fi
  (cd "$CURRENT_DIR" && cargo build --release) >/dev/null 2>&1 || {
    tmux display-message "tmux-palette: 'cargo build --release' failed in $CURRENT_DIR"
    exit 0
  }
  BIN="$CURRENT_DIR/target/release/tmux-palette"
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
