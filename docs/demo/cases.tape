# Install a missing tool, and a destructive command in the red box.
#   vhs docs/demo/cases.tape           (run `. docs/demo/setup.sh` once first)
# NOT recorded yet, and the install step needs a machine where `ncdu` is not installed and sudo
# works: do not record it on a machine you care about without reading the command in the box.
Output docs/pistasho-casos.gif
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

# an unknown tool: the box shows the exact command and waits for an explicit y
Ctrl+Space
Sleep 600ms
Type "see how much space each folder takes"
Enter
Sleep 3s
Escape
Sleep 1s

# a destructive command: the red box; Enter leaves it on the prompt, nothing runs
Ctrl+Space
Sleep 600ms
Type "format the usb as ext4"
Enter
Sleep 3s
Enter
Sleep 3s
Ctrl+U
Sleep 500ms
