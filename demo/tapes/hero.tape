# The README / landing-page clip: open the palette, jump to a pane by name,
# browse panes with the live preview, and switch theme.
Output docs/media/hero.gif
Output docs/media/hero.mp4
Source demo/tapes/settings.tape
Source demo/tapes/attach.tape

Sleep 1.2s
Ctrl+Space
Sleep 1.8s

# Panes surface inline as you type — jump straight to the dev server.
Type "vite"
Sleep 1.4s
Enter
Sleep 2s

# Find Pane: the whole tree, with each pane's live screen beside it.
Ctrl+Space
Sleep 600ms
Type "find"
Sleep 600ms
Enter
Sleep 1.4s
Up@700ms 7
Sleep 600ms
Screenshot docs/media/hero.png
Sleep 400ms # Screenshot grabs the next frame; keep the next key out of it
Down@700ms 1
Sleep 900ms
Enter
Sleep 1.6s

# Themes preview live as you move.
Ctrl+Space
Sleep 600ms
Type "theme"
Sleep 600ms
Enter
Sleep 900ms
Down@650ms 12
Sleep 700ms
Enter
Sleep 1.4s
Escape
Sleep 1.2s
