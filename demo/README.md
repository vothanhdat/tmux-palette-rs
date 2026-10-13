# Demo recordings

The GIFs and videos in [`docs/media/`](../docs/media) — shown in the README and
on the [website](https://vothanhdat.github.io/tmux-palette-rs/) — are real
recordings of the binary built from this repo, scripted with
[VHS](https://github.com/charmbracelet/vhs) so they can be re-recorded whenever
the UI changes.

```bash
demo/record.sh                 # re-record every clip
demo/record.sh hero themes     # or just some
```

Each clip writes `docs/media/<clip>.gif` (README), `.mp4` (website) and a
`.png` still (video poster / social preview). Commit them with the change that
made them stale.

## Requirements

- [`vhs`](https://github.com/charmbracelet/vhs#installation) — with its `ttyd`
  and `ffmpeg` dependencies
- `tmux`, `git`, `python3`, and a Rust toolchain (the script builds
  `target/release/tmux-palette`)
- **JetBrainsMono Nerd Font Mono** installed, or the palette's icons render as
  boxes ([nerd-fonts releases](https://github.com/ryanoasis/nerd-fonts/releases))

## How it works

| File | Role |
|------|------|
| `env/setup.sh` | Builds a throwaway workspace under `/tmp/tmux-palette-demo` — a fake `$HOME` with a few projects (one is a clone of this repo) — and starts a tmux server on a private socket (`tmux -L palette-demo`) with the palette bound to <kbd>Ctrl</kbd> <kbd>Space</kbd>, <kbd>Alt</kbd> <kbd>f</kbd>, <kbd>Alt</kbd> <kbd>p</kbd>, <kbd>Alt</kbd> <kbd>b</kbd> and <kbd>prefix</kbd> <kbd>:</kbd>. Your own tmux server and `~/.config` are never touched. |
| `env/tmux.conf`, `env/bashrc` | The look of that workspace: status bar, pane titles, prompt. |
| `env/bin/fake-log` | Seeded, colored dev-server / worker logs, so panes are alive and the Find Pane preview has something to update. |
| `env/config/` | Copied to the fake `~/.config/tmux-palette/` (plus `examples/git-branches.json`). |
| `tapes/settings.tape` | Shared size, font and theme for every clip. |
| `tapes/attach.tape` | Runs `setup.sh` and attaches, off camera. |
| `tapes/<clip>.tape` | The keystrokes of each clip. |

To poke at the workspace by hand: `demo/env/setup.sh && tmux -L palette-demo attach`,
then `demo/env/setup.sh --kill` when done.

VHS can't type every key: use `Alt+"="` rather than `Alt+=`, and bind
demo-only shortcuts in `setup.sh` when a tape needs a key VHS lacks.
