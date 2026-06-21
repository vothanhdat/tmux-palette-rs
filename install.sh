#!/usr/bin/env bash
# Install a prebuilt tmux-palette binary — no Rust toolchain required.
#
# Usage:
#   ./install.sh [DEST]
#
# Picks the prebuilt binary for this platform. If run from a checkout of the
# `stable` branch it uses the local dist/ binary; otherwise it downloads it
# from the stable branch of the origin remote. DEST defaults to
# ~/.local/bin/tmux-palette.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DEST="${1:-$HOME/.local/bin/tmux-palette}"

case "$(uname -s)/$(uname -m)" in
  Linux/x86_64) ASSET="tmux-palette-linux-x64" ;;
  Darwin/arm64) ASSET="tmux-palette-macos-arm64" ;;
  *)
    echo "No prebuilt binary for $(uname -s)/$(uname -m)." >&2
    echo "Build from source instead: cargo build --release" >&2
    exit 1
    ;;
esac

mkdir -p "$(dirname "$DEST")"

if [ -f "$SCRIPT_DIR/dist/$ASSET" ]; then
  echo "Installing $ASSET from local dist/ ..."
  cp "$SCRIPT_DIR/dist/$ASSET" "$DEST"
else
  # Derive owner/repo from the origin remote to download from the stable branch.
  REMOTE="$(git -C "$SCRIPT_DIR" remote get-url origin 2>/dev/null || true)"
  SLUG="$(printf '%s' "$REMOTE" | sed -E 's#^git@github\.com:##; s#^https://github\.com/##; s#\.git$##')"
  if [ -z "$SLUG" ]; then
    echo "No local dist/$ASSET and no GitHub origin remote to download from." >&2
    echo "Clone the stable branch first:  git clone --branch stable --depth 1 <repo>" >&2
    exit 1
  fi
  URL="https://raw.githubusercontent.com/$SLUG/stable/dist/$ASSET"
  echo "Downloading $URL ..."
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$URL" -o "$DEST"
  else
    wget -qO "$DEST" "$URL"
  fi
fi

chmod +x "$DEST"
echo "Installed: $DEST"
echo
echo "Add a binding to ~/.tmux.conf, e.g.:"
echo "  bind -n C-Space run-shell \"$DEST\""
echo "Then reload: tmux source-file ~/.tmux.conf"
