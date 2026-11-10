#!/bin/bash
# Installs Apple's SF Pro fonts from Apple's own SF-Pro.dmg (developer.apple.com/fonts)
# for this user, and makes them the system font. Apple's licence covers use on Apple
# devices, so the fonts are never stored in the repo: you supply the DMG.
#   install-sf-pro.sh [path/to/SF-Pro.dmg]    (default: ~/Downloads/SF-Pro.dmg)
set -euo pipefail
DMG=${1:-$HOME/Downloads/SF-Pro.dmg}
[ -f "$DMG" ] || { echo "No $DMG - download SF Pro from https://developer.apple.com/fonts/"; exit 1; }
command -v 7z >/dev/null || { echo "Needs 7z: sudo dnf install p7zip p7zip-plugins"; exit 1; }
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
7z x -y -o"$T/dmg" "$DMG" >/dev/null
PKG=$(find "$T/dmg" -name '*.pkg' | head -1)
7z x -y -o"$T/pkg" "$PKG" >/dev/null
PAYLOAD=$(find "$T/pkg" -name Payload | head -1)
mkdir -p "$T/out" && (cd "$T/out" && 7z x -so "$PAYLOAD" | cpio -idm 2>/dev/null)
F=$HOME/.local/share/fonts/SF-Pro
mkdir -p "$F"
find "$T/out" \( -name '*.otf' -o -name '*.ttf' \) -exec cp {} "$F/" \;
fc-cache -f "$F" >/dev/null
echo "Installed $(ls "$F" | wc -l) SF Pro font files into $F"
"$HOME/.local/share/cosmic-mac/apply-settings.sh" --fonts-only
