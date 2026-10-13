#!/usr/bin/env bash
# Re-record the demo clips in docs/media/ from the tapes in demo/tapes/.
#
#   demo/record.sh              # every clip
#   demo/record.sh hero themes  # just these
#
# Needs: vhs (https://github.com/charmbracelet/vhs, which brings ttyd + ffmpeg),
# tmux, git, python3, and JetBrainsMono Nerd Font installed (the palette's icons
# are Nerd Font glyphs). Each clip builds its own throwaway tmux workspace on a
# private socket (demo/env/setup.sh), so your own tmux session is untouched.
set -euo pipefail

cd "$(dirname "$0")/.."

for tool in vhs tmux git python3; do
  command -v "$tool" >/dev/null || { echo "record: '$tool' not found" >&2; exit 1; }
done
# grep reads to the end (no -q): quitting early would SIGPIPE fc-list, and
# pipefail would read that as "not installed".
if command -v fc-list >/dev/null && ! fc-list | grep -i "JetBrainsMono Nerd Font Mono" >/dev/null; then
  echo "record: JetBrainsMono Nerd Font Mono is not installed; icons will render as boxes" >&2
fi

cargo build --release --quiet

if [ $# -eq 0 ]; then
  set -- hero find-pane command-prompt themes custom-palettes
fi

mkdir -p docs/media
for clip in "$@"; do
  echo "==> $clip"
  vhs "demo/tapes/$clip.tape" >/dev/null # writes $clip.gif, .mp4 and a .png still
done

demo/env/setup.sh --kill
ls -lh docs/media
