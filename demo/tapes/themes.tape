# Switch Theme: every bundled theme previews live as you move through the list.
Output docs/media/themes.gif
Output docs/media/themes.mp4
Source demo/tapes/settings.tape
Source demo/tapes/attach.tape

Sleep 1s
Ctrl+Space
Sleep 800ms
Type "theme"
Sleep 800ms
Enter
Sleep 1s
Down@750ms 12
Sleep 600ms
Screenshot docs/media/themes.png
Sleep 400ms # Screenshot grabs the next frame; keep the next key out of it
Up@750ms 4
Sleep 600ms
Enter
Sleep 1.6s
Escape
Sleep 1s
