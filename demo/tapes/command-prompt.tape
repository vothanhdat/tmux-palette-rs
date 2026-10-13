# prefix + : — every tmux command, searchable and described; arguments expand
# once the name is complete.
Output docs/media/command-prompt.gif
Output docs/media/command-prompt.mp4
Source demo/tapes/settings.tape
Source demo/tapes/attach.tape

Sleep 1s
Ctrl+B
Type ":"
Sleep 1.5s
Down@500ms 5
Sleep 800ms
Type "pane"
Sleep 1.8s
Ctrl+U
Sleep 400ms
Type "split-window "
Sleep 2.2s
Screenshot docs/media/command-prompt.png
Sleep 400ms # Screenshot grabs the next frame; keep the next key out of it
Type "-v -l 35%"
Sleep 1s
Enter
Sleep 1.8s

Ctrl+B
Type ":"
Sleep 800ms
Type "renam"
Sleep 900ms
Tab
Sleep 900ms
Tab
Sleep 1.2s
Type " logs"
Sleep 900ms
Enter
Sleep 2s
