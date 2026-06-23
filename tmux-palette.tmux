#!/usr/bin/env bash
# TPM entry point for tmux-palette (Rust port).
# Sourced by tmux when the plugin is installed via tmux-plugins/tpm.
#
# Prefers a prebuilt binary shipped in dist/ (so installing from the `master`
# branch needs no Rust toolchain); downloads one if the clone has none, and
# falls back to `cargo build --release`.

set -eu

CURRENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Map the current platform to a prebuilt binary name, if one exists.
prebuilt_name() {
  case "$(uname -s)/$(uname -m)" in
    Linux/x86_64)        echo "tmux-palette-linux-x64" ;;
    Linux/aarch64)       echo "tmux-palette-linux-arm64" ;;
    Darwin/arm64)        echo "tmux-palette-macos-arm64" ;;
    *)                   echo "" ;;
  esac
}

# Repo to fetch a prebuilt from when dist/ has none (e.g. the plugin was cloned
# from a branch without prebuilts, like the default `master`). Derived from the
# clone's origin so forks fetch their own; overridable with @palette-repo,
# falling back to upstream.
palette_repo() {
  local repo remote
  repo="$(tmux show-option -gqv @palette-repo 2>/dev/null || true)"
  if [ -z "$repo" ]; then
    remote="$(git -C "$CURRENT_DIR" remote get-url origin 2>/dev/null || true)"
    case "$remote" in
      *github.com[:/]*)
        repo="${remote##*github.com}"   # drop scheme/host (and any user@)
        repo="${repo#[:/]}"             # drop leading : or /
        repo="${repo%.git}"
        ;;
    esac
  fi
  echo "${repo:-vothanhdat/tmux-palette-rs}"
}

# Download the prebuilt for this platform into dist/ — no toolchain required.
# Sets BIN on success; returns non-zero otherwise. Cached in dist/, so this
# runs at most once per install.
download_prebuilt() {
  [ -n "$PREBUILT" ] || return 1
  local repo ref url dest
  repo="$(palette_repo)"
  ref="$(tmux show-option -gqv @palette-ref 2>/dev/null || true)"
  ref="${ref:-master}"
  url="https://raw.githubusercontent.com/$repo/$ref/dist/$PREBUILT"
  dest="$CURRENT_DIR/dist/$PREBUILT"
  mkdir -p "$CURRENT_DIR/dist"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$dest" || return 1
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$dest" "$url" || return 1
  else
    return 1
  fi
  chmod +x "$dest" 2>/dev/null || true
  BIN="$dest"
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

# Keys bind in the root table (press them directly) by default. Set
# `@palette-prefix on` to bind them in the prefix table instead, so they fire
# as `<prefix> <key>` (e.g. `set -g @palette-prefix on` + `@palette-key p`).
USE_PREFIX="$(get_opt @palette-prefix 'off')"

palette_bind() {
  local key="$1"; shift
  case "$USE_PREFIX" in
    on|true|yes|1) tmux bind-key "$key" "$@" ;;   # prefix table
    *)             tmux bind-key -n "$key" "$@" ;; # root table (no prefix)
  esac
}

if [ "$PALETTE_KEY" != "off" ] && [ -n "$PALETTE_KEY" ]; then
  palette_bind "$PALETTE_KEY" run-shell "$BIN"
fi

if [ -n "$FIND_PANE_KEY" ]; then
  palette_bind "$FIND_PANE_KEY" run-shell "$BIN find-pane"
fi

if [ -n "$MOVE_PANE_KEY" ]; then
  palette_bind "$MOVE_PANE_KEY" run-shell "$BIN move-pane"
fi
