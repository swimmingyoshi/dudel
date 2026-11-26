#!/usr/bin/env bash
# Install Dudel for the current user, no root needed. Projects and preferences
# live elsewhere, so --uninstall leaves them in place.
# Usage: ./install.sh [--uninstall]
set -euo pipefail
cd "$(dirname "$0")"
bin="$HOME/.local/bin/dudel"
entry="${XDG_DATA_HOME:-$HOME/.local/share}/applications/dudel.desktop"
icon="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps/dudel.png"
case "${1:-}" in
    "")
        install -Dm755 dudel "$bin"
        install -Dm644 dudel.png "$icon"
        # Launchers may not have ~/.local/bin on PATH, so name the file exactly.
        mkdir -p "$(dirname "$entry")"
        sed "s|^Exec=.*|Exec=$bin|" dudel.desktop > "$entry"
        echo "Installed $bin and the Dudel launcher"
        ;;
    --uninstall)
        rm -f "$bin" "$entry" "$icon"
        echo 'Removed Dudel and its launcher; projects and preferences were kept'
        ;;
    *) echo 'Usage: install.sh [--uninstall]' >&2; exit 2;;
esac
