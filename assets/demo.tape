# snp terminal demo recorded with [vhs](https://github.com/charmbracelet/vhs).
#
# Requirements:
#   brew install vhs
#
# Usage:
#   vhs assets/demo.tape            # regenerates assets/demo.gif
#   vhs publish assets/demo.tape    # uploads to vhs.charm.sh and prints URL

Output assets/demo.gif

Set Shell bash
Set FontSize 14
Set Width 1100
Set Height 620
Set Theme "Catppuccin Mocha"
Set Padding 16

# Show version briefly
Type "snp --version"
Sleep 800ms
Enter
Sleep 1s

# Launch the TUI
Type "snp run"
Sleep 500ms
Enter
Sleep 2s

# Fuzzy filter to ssh
Type "ssh"
Sleep 1.5s
Enter

# Variable prompt appears
Sleep 1s
# Accept default for user
Enter
Sleep 400ms
# Override host
Type "prod.example.com"
Sleep 600ms
Enter
Sleep 1.5s

# Quit TUI
Escape
Escape
Type "q"
Enter
Sleep 800ms
