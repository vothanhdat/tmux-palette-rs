#!/usr/bin/env sh
# Install a prebuilt tmux-palette binary — no Rust toolchain required.
#
# Usage:
#   ./install.sh [DEST]
#   curl -fsSL https://raw.githubusercontent.com/vothanhdat/tmux-palette-rs/stable/install.sh | sh
#
# Picks the prebuilt binary for this platform. If run from a checkout of the
# `stable` branch it uses the local dist/ binary; otherwise it downloads it
# from GitHub. DEST defaults to ~/.local/bin/tmux-palette.

set -eu

DEFAULT_REPO="vothanhdat/tmux-palette-rs"
DEFAULT_REF="stable"
DEST="$HOME/.local/bin/tmux-palette"
REPO="${TMUX_PALETTE_REPO:-}"
REF="${TMUX_PALETTE_REF:-$DEFAULT_REF}"

usage() {
  cat <<EOF
Install tmux-palette prebuilt binary.

Usage:
  ./install.sh [DEST]
  ./install.sh --dest /usr/local/bin/tmux-palette
  curl -fsSL https://raw.githubusercontent.com/$DEFAULT_REPO/$DEFAULT_REF/install.sh | sh
  curl -fsSL https://raw.githubusercontent.com/$DEFAULT_REPO/$DEFAULT_REF/install.sh | sh -s -- --dest /usr/local/bin/tmux-palette

Options:
  -d, --dest PATH     Install path (default: $HOME/.local/bin/tmux-palette)
      --repo OWNER/REPO
                      GitHub repo to download from (default: $DEFAULT_REPO)
      --ref REF       Git ref/branch to download from (default: $DEFAULT_REF)
  -h, --help          Show this help

Environment:
  TMUX_PALETTE_REPO   Same as --repo
  TMUX_PALETTE_REF    Same as --ref
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    -d|--dest)
      if [ "$#" -lt 2 ]; then
        echo "install.sh: $1 requires a path" >&2
        exit 2
      fi
      DEST="$2"
      shift 2
      ;;
    --repo)
      if [ "$#" -lt 2 ]; then
        echo "install.sh: --repo requires OWNER/REPO" >&2
        exit 2
      fi
      REPO="$2"
      shift 2
      ;;
    --ref)
      if [ "$#" -lt 2 ]; then
        echo "install.sh: --ref requires a git ref" >&2
        exit 2
      fi
      REF="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      echo "install.sh: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      DEST="$1"
      shift
      ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "$0")" 2>/dev/null && pwd || pwd)"

case "$(uname -s)/$(uname -m)" in
  Linux/x86_64) ASSET="tmux-palette-linux-x64" ;;
  Linux/aarch64) ASSET="tmux-palette-linux-arm64" ;;
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
  if [ -z "$REPO" ] && command -v git >/dev/null 2>&1; then
    REMOTE="$(git -C "$SCRIPT_DIR" remote get-url origin 2>/dev/null || true)"
    REPO="$(printf '%s' "$REMOTE" | sed -E 's#^git@github\.com:##; s#^https://github\.com/##; s#\.git$##')"
  fi
  if [ -z "$REPO" ]; then
    REPO="$DEFAULT_REPO"
  fi

  URL="https://raw.githubusercontent.com/$REPO/$REF/dist/$ASSET"
  echo "Downloading $URL ..."
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$URL" -o "$DEST"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$DEST" "$URL"
  else
    echo "install.sh: need curl or wget to download $ASSET" >&2
    exit 1
  fi
fi

chmod +x "$DEST"
echo "Installed: $DEST"
echo
echo "Add a binding to ~/.tmux.conf, e.g.:"
echo "  bind -n C-Space run-shell \"$DEST\""
echo "Then reload: tmux source-file ~/.tmux.conf"
