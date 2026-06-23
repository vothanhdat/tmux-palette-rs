# tmux-palette (Rust)

A command palette for tmux. This is a **Rust port** of
[eduwass/tmux-palette](https://github.com/eduwass/tmux-palette) (originally
TypeScript/Bun). It compiles to a single static binary — no runtime, no Bun, no
Node — and opens quickly enough to use as a regular tmux key binding.

Type a few letters, pick a command, hit enter: split a pane, jump to a window,
detach a session, open a popup tool, or switch to a custom palette. User config
lives in `~/.config/tmux-palette/*.json`, so local changes survive repo updates.

**Commands** — main palette for panes, windows, sessions, and built-in tmux actions.
**Themes + plugins** — theme picker with live preview, plus custom palettes powered by shell commands.

## Why a Rust port?

- **One self-contained binary.** Bind a key straight to the binary — no wrapper
  script and no separate runtime to install. The binary is both the launcher
  *and* the palette: with no special environment it opens a `display-popup`
  running itself, then dispatches your chosen command after the popup closes.
- **Behavior-compatible.** Same palettes, same fuzzy matching, same theming, same
  `~/.config/tmux-palette/*.json` config files, same keybindings. The TypeScript
  test suite was ported alongside the code (`cargo test`).
- **Fast startup**, suitable for frequent use from a key binding.

## Highlights

- **Fast startup** — designed for frequent use from a tmux key binding
- **Inline pane search** — just start typing in the main palette to jump to any
  pane across sessions/windows; live panes surface alongside commands (ranked by
  the same fuzzy matcher) without first opening *Find Pane*. They stay hidden
  until you type, so the resting palette is unchanged. Search matches the pane
  title, session/window, running command, detected agent, and path.
- **Custom palettes** — define your own with a single JSON file, bind to any key
- **Hide built-ins** — declutter the default palette via `hidden.json`
- **Mobile-aware** — auto-fullscreens on narrow terminals (Moshi / Blink on iOS)
- **Curated themes** — 13 built-in themes (Shades of Purple, Dracula, Tokyo Night,
  Catppuccin, Gruvbox, Nord, Solarized, and a transparent `Terminal` theme that
  follows your terminal colors). Pick one with live preview, or drop your own.
- **Popup tools** — use `{ "popup": "htop" }` to open `btop`, `lazygit`, log tails,
  or `fzf` scripts in a tmux popup
- **Scriptable sources** — point a palette at a shell command that prints JSON or
  one item per line. Examples live in [`examples/`](examples)
- **No fork required** — every customization lives in `~/.config/tmux-palette/*.json`

## Requirements

- tmux 3.4+ recommended (`display-popup -E` support; tested on 3.3a)
- To install a prebuilt binary: nothing else (see below).
- To build from source: a Rust toolchain (`cargo`, stable) — see https://rustup.rs
- Optional tools for examples only: `gh`, `jq`, `docker`, `npm`, `git`, etc.

## Install

### One-line install — prebuilt, no toolchain

The `master` branch ships prebuilt binaries in [`dist/`](dist) (published by CI),
so you can install without Rust. Prebuilts are provided for **Linux x86_64**,
**Linux arm64**, and **macOS arm64 (Apple Silicon)**; other platforms fall back
to a source build. Linux prebuilts are built with musl so they do not require a
specific host glibc version.

```bash
curl -fsSL https://raw.githubusercontent.com/vothanhdat/tmux-palette-rs/master/install.sh | sh
```

Install somewhere else:

```bash
curl -fsSL https://raw.githubusercontent.com/vothanhdat/tmux-palette-rs/master/install.sh | sh -s -- --dest /usr/local/bin/tmux-palette
```

Then bind it (the installer prints this line for you):

```tmux
bind -n C-Space run-shell "~/.local/bin/tmux-palette"
```

Reload tmux:

```bash
tmux source-file ~/.tmux.conf
```

You can also download the installer first and inspect its options:

```bash
curl -fsSLO https://raw.githubusercontent.com/vothanhdat/tmux-palette-rs/master/install.sh
sh install.sh --help
```

### From a checkout

```bash
git clone --depth 1 git@github.com:vothanhdat/tmux-palette-rs.git ~/Sites/tmux-palette
cd ~/Sites/tmux-palette
./install.sh                     # copies the right binary to ~/.local/bin/tmux-palette
# or: ./install.sh /usr/local/bin/tmux-palette
```

The prebuilt binary is also usable directly without the installer, e.g.
`~/Sites/tmux-palette/dist/tmux-palette-linux-x64`.

**Via TPM, no build:** the default branch (`master`) ships the prebuilts in
`dist/`, so the plugin's loader auto-detects the right one and skips compiling.
If you install from a branch without prebuilts, the loader downloads one; `cargo`
is only used as a last-resort fallback when no prebuilt matches your platform.

### Manual (build from source)

```bash
git clone git@github.com:vothanhdat/tmux-palette-rs.git ~/Sites/tmux-palette
cd ~/Sites/tmux-palette
cargo build --release
# binary: ./target/release/tmux-palette
```

Bind it to a tmux key in your `.tmux.conf` — `Ctrl+Space` gives the most
"Raycast-feel" since it skips the prefix:

```tmux
bind -n C-Space run-shell "~/Sites/tmux-palette/target/release/tmux-palette"
```

Or go through the tmux prefix:

```tmux
bind p run-shell "~/Sites/tmux-palette/target/release/tmux-palette"
```

Optionally bind the focused palettes directly:

```tmux
bind -n M-f run-shell "~/Sites/tmux-palette/target/release/tmux-palette find-pane"
bind -n M-m run-shell "~/Sites/tmux-palette/target/release/tmux-palette move-pane"
```

Reload: `tmux source-file ~/.tmux.conf` and hit your binding.

### Via TPM (Tmux Plugin Manager)

Add to your `.tmux.conf`:

```tmux
set -g @plugin 'vothanhdat/tmux-palette-rs'
set -g @palette-key 'C-Space'             # optional, default: C-Space (no-prefix)
set -g @palette-find-pane-key 'M-f'       # optional, no binding by default
set -g @palette-move-pane-key 'M-m'       # optional, no binding by default
set -g @palette-prefix 'off'              # optional, 'on' = bind behind the prefix
```

Then `prefix + I`. TPM clones the default branch (`master`), which ships the
prebuilts in `dist/`, and binds the keys for you — no build. If your platform has
no prebuilt, the loader downloads one (or runs `cargo build --release` as a last
resort). Set `@palette-key 'off'` to skip the main binding and bind it yourself.

By default the keys are bound in the **root table** (press them directly, no
prefix). Set `@palette-prefix 'on'` to bind them in the **prefix table** instead,
so they fire as `<prefix> <key>`:

```tmux
set -g @palette-prefix 'on'
set -g @palette-key 'p'                   # now: prefix + p
```

### Install the binary on PATH (alternative)

```bash
cargo install --path .
# then: bind -n C-Space run-shell "tmux-palette"
```

## Usage

- **Type** to filter. Multi-word search is supported (`split horiz`).
- **Up/Down arrows** or **Ctrl+P / Ctrl+N** to move selection.
- **PageUp / PageDown** jump 10 rows.
- **Enter** to run the selected command.
- **Esc** to cancel (or pop back one level in a nested palette).
- **Mouse** works too — click rows, scroll the wheel, click `esc`.

**Auto-aliases**: initials of multi-word titles match automatically. Type `nw`
for "New Window", `cs` for "Choose Session", `sh` for "Split Horizontal".

**Jump to a pane**: start typing and matching panes appear inline — by title,
session/window, running command, agent, or path (e.g. type a project name or
`nvim`). Pick one to switch straight to it. The dedicated **Find Pane** entry
still opens the full session/window/pane tree.

## How it works (the trick)

The binary, run with no special environment, opens a `tmux display-popup`
running *itself* (with `TMUX_PALETTE_CMD` set so it enters interactive mode).
When you pick an item, the palette writes the encoded command to a tempfile and
exits. The launcher *then* reads the tempfile and runs the command — *after* the
popup is gone. This matters because interactive tmux commands like
`confirm-before` and `command-prompt` need stdin, which is captured by the popup
while it's open.

`{ tmux }` actions dispatch after the popup closes; `{ popup }` actions re-open a
sized popup and relaunch the palette afterward; `{ palette }` actions navigate
in-process (a Raycast-style back stack).

## Customize

Drop-in user config lives in `~/.config/tmux-palette/` (honoring
`XDG_CONFIG_HOME`). One JSON file per concern — no source edits, no fork,
survives upstream pulls. **All of the following behave identically to the
original**, so the upstream docs apply:

- `commands.json` — append your own items (action types: `{ "tmux": … }`,
  `{ "shell": … }`, `{ "popup": … }`, `{ "palette": … }`)
- `hidden.json` — array of item titles to hide from the main palette
- `theme.json` — `{ "name": "tokyo-night" }` or a full/partial color override
- `themes/<slug>.json` — custom themes (appear in the switcher)
- `palettes/<name>.json` — brand-new palettes; bind a key to their name
- `sizing.json` — popup dimensions, borders, mobile width, ESC behavior
- `shortcuts.json` — custom shortcut labels
- `aliases.json` — extra visible alias chips

### Custom palettes & plugins

Path: `~/.config/tmux-palette/palettes/<name>.json`. Bind any key to its name:

```tmux
bind -n M-q run-shell "~/Sites/tmux-palette/target/release/tmux-palette my-favs"
```

A palette can pull items `from` the main palette, from a `fromCategory`, define
inline `items`, or run a shell `command` whose stdout becomes the palette —
either a JSON array of `Item` objects, or one item per line (fzf-style) with an
`action` template where `{}` is replaced by the selected line. Ready-to-use
examples (git branches, GitHub PRs, Docker logs, npm scripts, file picker) live
in [`examples/`](examples) — copy one into `palettes/` and bind a key.

### Category hotkeys

```tmux
bind -n M-t run-shell "~/Sites/tmux-palette/target/release/tmux-palette commands --category=Tools"
```

### Themes

Open the palette and pick **Switch Theme...** (under *Appearance*) — every theme
live-previews as you arrow through the list; Enter saves it and returns you to
the previous palette, Esc cancels. The switcher writes `theme.json` for you.

Color fields accept hex (`#1a1b26`), `transparent` (use the terminal default),
or an ANSI palette name (`blue`, `bright-black`, …) so the palette can track your
terminal's own colorscheme. See the bundled `terminal` theme.

## Differences from the original

- Distributed as a compiled binary; install with `cargo build` instead of
  `bun install`. The bash wrapper is gone — the binary is its own launcher.
- A `TMUX_PALETTE_CONFIG_DIR` env var can override the config directory (handy
  for tests).
- The search input accepts pasted / coalesced multi-character input (the
  original inserted one character per keystroke event).
- An unknown theme name in `theme.json` logs a warning and falls back to the
  default instead of aborting.
- The main palette inlines live panes as you type, so you can jump to a pane
  without first entering *Find Pane* (the original only exposed panes through the
  dedicated sub-palette).

## Project layout

```
src/
  main.rs            launcher / run / measure entry point
  cli.rs             palette resolution, plugin commands, popup sizing
  palette.rs         interactive raw-mode TUI loop
  render.rs          row composition + ANSI styling
  fuzzy.rs           fuzzy matching + ranking
  text.rs            display width / truncation / auto-alias
  theme.rs           theme resolution + ANSI/tmux color translation
  themes_bundled.rs  the curated themes
  dispatch.rs        action encoding for the launcher
  tmux.rs            tmux command helpers
  user_config.rs     ~/.config/tmux-palette/*.json loaders
  raw.rs             termios raw mode, terminal size, signal handling
  palettes/          commands, find-pane, move-pane, themes
```

Run the tests with `cargo test`.

### Branches

- **`master`** — the release branch. Carries the prebuilt binaries in `dist/`
  (published by CI) and is what the installer, README, and TPM pull from. It is
  the repo's default branch, so `set -g @plugin 'vothanhdat/tmux-palette-rs'`
  installs with no build.
- **`dev`** — where development happens (source only, no `dist/`). Open changes
  here; CI builds and tests every push. Merge `dev` → `master` to cut a release,
  and CI republishes the prebuilts.

## License

MIT. Original project by Eduard Wassermann; see [LICENSE](LICENSE).
