# The four steps of ADR 0001: answer, alternatives, filter and replace, explore.
#   vhs docs/demo/question.tape        (needs vhs and ttyd; run `. docs/demo/setup.sh` once first)
# NOT recorded yet: vhs was not installed when this was written, so the key names and the waits
# are untested; adjust them to your vhs version.
Output docs/pistasho-modo-pregunta.gif
Require zsh
Set Shell "zsh"
Set FontSize 16
Set Width 1100
Set Height 620
Set TypingSpeed 55ms
Set Padding 16

Hide
Type "PS1='❯ '; . docs/demo/setup.sh; clear"
Enter
Sleep 2s
Show

# 1. Ctrl-Space, a question, Enter: the best command lands on the prompt
Ctrl+Space
Sleep 600ms
Type "how long has this machine been up"
Sleep 400ms
Enter
Sleep 2500ms

# 2. Not the one? Ctrl-Space again: other options for the same question
Ctrl+Space
Sleep 1500ms

# 3. Type to filter, pick one, Enter replaces the command
Type "uptime"
Sleep 800ms
Down
Sleep 600ms
Enter
Sleep 2s
Ctrl+U
Sleep 400ms

# 4. Ctrl-Space then Tab on an empty line: explore your cheats, history and tldr
Ctrl+Space
Sleep 500ms
Tab
Sleep 2500ms
Escape
Sleep 500ms
