# Custom palettes: one JSON file each, fed by any shell command.
Output docs/media/custom-palettes.gif
Output docs/media/custom-palettes.mp4
Source demo/tapes/settings.tape
Source demo/tapes/attach.tape

# Start in the shell pane, not the editor.
Hide
Ctrl+B
Right
Type "clear"
Enter
Sleep 500ms
Show

Sleep 800ms
Type@35ms "cat ~/.config/tmux-palette/palettes/git-branches.json"
Sleep 400ms
Enter
Sleep 3s

# Alt+b: branches from `git branch`, Enter checks one out in this pane.
Alt+b
Sleep 1.5s
Screenshot docs/media/custom-palettes.png
Sleep 400ms # Screenshot grabs the next frame; keep the next key out of it
Type "inl"
Sleep 1s
Enter
Sleep 2.5s

# Alt+p: projects from `ls ~/code`, Enter switches to (or creates) its session.
Alt+p
Sleep 1.5s
Type "web"
Sleep 1s
Enter
Sleep 2.5s
