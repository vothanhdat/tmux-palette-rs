# Find Pane: tree on the left, the highlighted pane's live screen on the right;
# filter by title / command / path, then resize from the keyboard.
Output docs/media/find-pane.gif
Output docs/media/find-pane.mp4
Source demo/tapes/settings.tape
Source demo/tapes/attach.tape

Sleep 1s
Alt+f
Sleep 1.5s
Up@800ms 4
Sleep 800ms
Screenshot docs/media/find-pane.png
Sleep 400ms # Screenshot grabs the next frame; keep the next key out of it
Type "work"
Sleep 1.5s
Enter
Sleep 2s

# Alt+= / Alt+- grow and shrink the popup.
Alt+f
Sleep 1.2s
Alt+"="
Sleep 1.2s
Alt+"="
Sleep 1.5s
Alt+"-"
Sleep 1.2s
Escape
Sleep 1s
