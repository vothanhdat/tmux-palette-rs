# A custom palette is one JSON file around a shell command: here `git branch`,
# and Enter checks the branch out in the pane.
Output target/short/6-custom.mp4
Source demo/short/settings.tape
Source demo/short/attach.tape

Hide
Ctrl+B
Down
Ctrl+B
Type "z"
Type "tmux select-pane -T shell; clear; cat ~/.config/tmux-palette/palettes/git-branches.json"
Enter
Sleep 800ms
Show
Sleep 2.4s
Alt+b
Sleep 1.3s
Type "inl"
Sleep 900ms
Enter
Sleep 2.2s
