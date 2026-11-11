# Shared settings, sourced by the other tapes. Run from the repo root after
# `cargo build --release`: `vhs vhs/hero.tape` etc.
Set Shell bash
Set FontFamily "Hack Nerd Font Mono"
Set FontSize 16
Set Width 960
Set Height 480
Set Padding 20
Set Framerate 25
Set TypingSpeed 40ms
Set Theme { "name": "tokyo-night", "background": "#1a1b26", "foreground": "#c0caf5", "cursor": "#c0caf5", "selection": "#33467c", "black": "#15161e", "red": "#f7768e", "green": "#9ece6a", "yellow": "#e0af68", "blue": "#7aa2f7", "magenta": "#bb9af7", "cyan": "#7dcfff", "white": "#a9b1d6", "brightBlack": "#414868", "brightRed": "#f7768e", "brightGreen": "#9ece6a", "brightYellow": "#e0af68", "brightBlue": "#7aa2f7", "brightMagenta": "#bb9af7", "brightCyan": "#7dcfff", "brightWhite": "#c0caf5" }

# Only the built-in themes: no config (fresh HOME), no Omarchy themes from
# this machine. Each tape then sets its flags in a hidden alias so the
# command shown stays short.
Hide
Type `export PATH="$PWD/target/release:$PATH" HOME=$(mktemp -d) XDG_STATE_HOME=/nonexistent OMARCHY_PATH=/nonexistent PS1='$ '; clear`
Enter
Sleep 500ms
Show
