# prefix + : — every tmux command, searchable and described. Tab completes the
# highlighted match into a pill; its flags expand, each explained and ticked off
# as you type it, and a strip says what the command does and which keys run it.
# Backspace on a pill takes the whole pick back.
Output docs/media/command-prompt.gif
Output docs/media/command-prompt.mp4
Source demo/tapes/settings.tape
Source demo/tapes/attach.tape

Sleep 1s
Ctrl+B
Type ":"
Sleep 1.5s
Down@900ms 4
Sleep 700ms
Type "spl"
Sleep 1.1s
Tab
Sleep 2s
Type "-h"
Sleep 900ms
Type " -l 35%"
Sleep 1.8s
Screenshot docs/media/command-prompt.png
Sleep 400ms # Screenshot grabs the next frame; keep the next key out of it
Enter
Sleep 1.8s

Ctrl+B
Type ":"
Sleep 800ms
Type "ren"
Sleep 900ms
Down
Sleep 700ms
Tab
Sleep 1.3s
Backspace
Sleep 1.1s
Down
Sleep 700ms
Tab
Sleep 1.1s
Type "logs"
Sleep 900ms
Enter
Sleep 2s
