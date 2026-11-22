#!/bin/sh
# Prepares a clean environment for recording the demos with vhs: its own config, data and cheats,
# so the recording shows nothing from your machine (no history, no real cheats).
#
#   . docs/demo/setup.sh        # exports XDG_* and HISTFILE for this shell, builds the index once
#
# Needs network the first time (tldr and the embedding model, ~5 min). NOT run by CI.
demo="${PISTASHO_DEMO_DIR:-/tmp/pistasho-demo}"
mkdir -p "$demo/config/pistasho" "$demo/data"
export XDG_CONFIG_HOME="$demo/config" XDG_DATA_HOME="$demo/data" HISTFILE=/dev/null
cp "$(dirname "$0")/../../examples/cheats.toml" "$demo/config/pistasho/cheats.toml"
printf 'language = "en"\nhistory = false\n' > "$demo/config/pistasho/config.toml"
[ -f "$demo/data/pistasho/pistasho.db" ] || pistasho update
eval "$(pistasho init zsh)"
