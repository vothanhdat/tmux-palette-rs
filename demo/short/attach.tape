# Build the demo workspace off camera and attach. The code window is stacked
# (editor over git log) instead of side by side, to suit the narrow terminal.
Hide
Type "demo/env/setup.sh >/dev/null && tmux -L palette-demo select-layout -t tmux-palette:code main-horizontal && clear && tmux -L palette-demo attach -t tmux-palette"
Enter
Sleep 4s
Show
