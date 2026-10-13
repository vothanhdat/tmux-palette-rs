#!/usr/bin/env bash
# Build the throwaway tmux workspace the demo tapes record, on a private socket
# so it never touches your own tmux server or config.
#
#   demo/env/setup.sh            # (re)create the workspace
#   tmux -L palette-demo attach  # look around (the tapes do this for you)
#   demo/env/setup.sh --kill     # tear it down
#
# Everything lives under $DEMO_ROOT (default /tmp/tmux-palette-demo): a fake
# $HOME with a few projects and its own ~/.config/tmux-palette.
set -euo pipefail

DEMO="$(cd "$(dirname "$0")/.." && pwd)"
REPO="$(cd "$DEMO/.." && pwd)"
SOCK=palette-demo
ROOT="${DEMO_ROOT:-/tmp/tmux-palette-demo}"
BIN="${TMUX_PALETTE_BIN:-$REPO/target/release/tmux-palette}"

t() { tmux -L "$SOCK" "$@"; }

t kill-server 2>/dev/null && sleep 0.5 # let its panes' programs exit
[ "${1:-}" = "--kill" ] && exit 0

[ -x "$BIN" ] || { echo "no binary at $BIN — run: cargo build --release" >&2; exit 1; }

rm -rf "$ROOT"
H="$ROOT/home"
mkdir -p "$H/code" "$H/dotfiles"

# Palette config at the real default path under the fake $HOME, so theme
# switches don't leak out and on-camera paths look like yours would.
mkdir -p "$H/.config"
cp -R "$DEMO/env/config" "$H/.config/tmux-palette"
cp "$REPO/examples/git-branches.json" "$H/.config/tmux-palette/palettes/"

# --- projects ---------------------------------------------------------------

git_init() {
  git -C "$1" init -q -b main
  git -C "$1" -c user.name=demo -c user.email=demo@example.com add -A
  git -C "$1" -c user.name=demo -c user.email=demo@example.com commit -qm "$2"
}

# This repository itself, so the editor and `git log` panes show real code.
git clone -q --shared "$REPO" "$H/code/tmux-palette"
TP="$H/code/tmux-palette"
git -C "$TP" checkout -q -B main
for b in $(git -C "$TP" for-each-ref --format='%(refname:short)' refs/heads); do
  [ "$b" = main ] || git -C "$TP" branch -qD "$b"
done
for b in feat/inline-panes fix/popup-resize docs/demo; do
  git -C "$TP" branch -q "$b"
done

mkdir -p "$H/code/api/src/routes"
cat >"$H/code/api/package.json" <<'EOF'
{
  "name": "api",
  "private": true,
  "scripts": {
    "dev": "tsx watch src/server.ts",
    "test": "vitest run",
    "lint": "eslint src",
    "db:migrate": "prisma migrate dev",
    "db:seed": "tsx scripts/seed.ts"
  }
}
EOF
printf 'export const router = new Router();\n' >"$H/code/api/src/routes/users.ts"
printf 'import { router } from "./routes/users";\n' >"$H/code/api/src/server.ts"
git_init "$H/code/api" "api: users routes"

mkdir -p "$H/code/web/src/components"
cat >"$H/code/web/src/App.tsx" <<'EOF'
import { useMemo, useState } from "react";
import { Palette } from "./components/Palette";
import { PaneTree } from "./components/PaneTree";
import { fuzzy } from "./lib/fuzzy";

type Pane = { id: string; title: string; session: string; path: string };

export default function App({ panes }: { panes: Pane[] }) {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);

  const matches = useMemo(
    () => fuzzy(panes, query, (p) => [p.title, p.session, p.path]),
    [panes, query],
  );

  return (
    <main className="app">
      <Palette
        query={query}
        onQuery={setQuery}
        items={matches}
        selected={selected}
        onMove={(d) => setSelected((s) => clamp(s + d, 0, matches.length - 1))}
      />
      <PaneTree panes={matches} highlight={matches[selected]?.id} />
    </main>
  );
}

const clamp = (n: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, n));
EOF
git_init "$H/code/web" "web: palette shell"

printf 'set -g mouse on\n' >"$H/dotfiles/tmux.conf"
git_init "$H/dotfiles" "dotfiles"

cat >"$ROOT/vimrc" <<'EOF'
set nocompatible termguicolors number laststatus=0 noshowmode noruler noswapfile nowrap
set shortmess+=I
syntax on
colorscheme habamax
EOF

# --- tmux server ------------------------------------------------------------

# The server copies this environment, so every pane and palette popup sees it.
export HOME="$H"
export LANG=C.UTF-8 LC_ALL=C.UTF-8 # or tmux mangles the agent's ✳ title
export TMUX_PALETTE_DEMO="$DEMO"
unset TMUX_PALETTE_CONFIG_DIR XDG_CONFIG_HOME
export EDITOR="vim -u $ROOT/vimrc"
SHELL_CMD="bash --rcfile $DEMO/env/bashrc -i"

title() { t select-pane -t "$1" -T "$2"; }
run() { t send-keys -t "$1" "$2" Enter; }

t -f "$DEMO/env/tmux.conf" new-session -d -s tmux-palette -n code -c "$H/code/tmux-palette" -x 160 -y 45 "$SHELL_CMD"
t set -g default-command "$SHELL_CMD"

# Palette keys, as the TPM plugin would bind them.
t bind -n C-Space run-shell "$BIN"
t bind -n M-f run-shell "$BIN find-pane"
t bind -n M-p run-shell "$BIN projects"
t bind -n M-b run-shell "$BIN git-branches"
t bind : run-shell "$BIN command-prompt"

# tmux-palette: the editor + history, and an agent working on a branch.
run tmux-palette:code "clear; $EDITOR src/fuzzy.rs"
title tmux-palette:code.1 "fuzzy.rs"
t split-window -h -l 45% -t tmux-palette:code -c "$H/code/tmux-palette"
run tmux-palette:code.2 "clear; git --no-pager log --no-merges -n 40 --format='%C(yellow)%h%C(reset) %<(44,trunc)%s'"
title tmux-palette:code.2 "git log"
t select-pane -t tmux-palette:code.1

t new-window -t tmux-palette -n agent -c "$H/code/tmux-palette"
run tmux-palette:agent "clear; git show --stat --color HEAD~1 | head -30"
title tmux-palette:agent.1 "✳ Record demo tapes"

# api: server + worker logs, and a shell.
t new-session -d -s api -n server -c "$H/code/api"
run api:server "clear; fake-log api"
title api:server.1 "api server"
t split-window -v -l 40% -t api:server -c "$H/code/api"
run api:server.2 "clear; fake-log jobs"
title api:server.2 "worker"
t new-window -t api -n shell -c "$H/code/api"
run api:shell "clear; git log --oneline --color; ls -la src"
title api:shell.1 "api shell"

# web: the dev server and the editor.
t new-session -d -s web -n dev -c "$H/code/web"
run web:dev "clear; fake-log web"
title web:dev.1 "vite"
t new-window -t web -n editor -c "$H/code/web"
run web:editor "clear; $EDITOR src/App.tsx"
title web:editor.1 "App.tsx"

t new-session -d -s dotfiles -n shell -c "$H/dotfiles"
run dotfiles:shell "clear; ls -la"
title dotfiles:shell.1 "dotfiles"

t select-window -t tmux-palette:code
sleep 1.5 # let the logs and editors draw before anything attaches
echo "demo workspace ready: tmux -L $SOCK attach -t tmux-palette"
