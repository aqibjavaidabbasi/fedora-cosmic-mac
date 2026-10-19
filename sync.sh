#!/bin/bash
# Copies the live COSMIC customizations from $HOME into this repo so git can
# track them. Run after any change, then commit and push (update.sh does this for you).
set -euo pipefail
R=$(cd "$(dirname "$0")" && pwd)
S=$R/snapshot/home
M=$HOME/.local/share/cosmic-mac

mkdir -p "$S/.config/systemd/user" "$S/.config/autostart" "$S/.local/bin" \
         "$S/.local/state" "$S/.local/share/applications" "$S/.local/share/icons"

rsync -a --delete "$HOME/.config/cosmic" "$HOME/.config/cosmic-mac" "$S/.config/"
cp -a "$HOME/.config/cosmic-initial-setup-done" "$S/.config/" 2>/dev/null || true
cp -a "$HOME"/.config/systemd/user/cosmic-mac-* "$S/.config/systemd/user/"
cp -a "$HOME/.config/autostart/cosmic-maximize-to-workspace.desktop" "$S/.config/autostart/"
rsync -a --delete "$HOME/.local/state/cosmic" "$HOME/.local/state/cosmic-comp" "$S/.local/state/"
# Toolkit: scripts, patches, saved upstream diffs, helper app sources.
# Not: binaries, upstream checkouts, and per-machine build stamps/logs.
rsync -a --delete --exclude bin/ --exclude build/ --exclude __pycache__/ --exclude target/ \
    --exclude .lock --exclude '*built-version' --exclude rebuild.log --exclude '*.orig' \
    "$M" "$S/.local/share/"
for f in "$HOME"/.local/bin/cosmic-*; do                # launcher scripts only, not binaries
    file -b "$f" | grep -q text && cp -a "$f" "$S/.local/bin/"
done
cp -a "$HOME"/.local/share/applications/com.aqib.*.desktop "$S/.local/share/applications/"
rsync -a --delete "$HOME/.local/share/icons/MacTahoe-COSMIC" "$S/.local/share/icons/"
cp -a "$HOME/.bash_profile" "$S/.bash_profile"

{ echo "# Installed COSMIC packages (rpm)"; rpm -qa 'cosmic*' | sort
  echo; echo "# Flatpak applets (remote cosmic)"
  flatpak list --app --columns=application,version,origin | grep -i -E 'cosmic|applet|gpu_screen_recorder' || true
  echo; echo "# Third-party themes in ~/.local/share/icons (not stored here; install.sh downloads them)"
  echo "MacTahoe, MacTahoe-dark, MacTahoe-light  https://github.com/vinceliuice/MacTahoe-icon-theme"
  echo "WhiteSur, WhiteSur-dark, WhiteSur-light   https://github.com/vinceliuice/WhiteSur-icon-theme"
  echo "WhiteSur-cursors                         https://github.com/vinceliuice/WhiteSur-cursors"
} > "$R/MANIFEST.txt"

git -C "$R" status --short
