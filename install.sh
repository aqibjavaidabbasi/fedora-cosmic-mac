#!/bin/bash
# Installs the macOS-style COSMIC setup on a Fedora machine for the current user.
#
#   ./install.sh            Mac look and features, keeping your own wallpaper, dock apps etc.
#   ./install.sh --restore  Also restore every COSMIC setting saved in this repo (exact copy of
#                           the original desktop; meant for reinstalling the owner's machine)
#
# Needs sudo once (packages). The first build takes about 25-40 minutes.
set -euo pipefail
R=$(cd "$(dirname "$0")" && pwd)
S=$R/snapshot/home
D=$HOME/.local/share/cosmic-mac
RESTORE=; [ "${1:-}" = "--restore" ] && RESTORE=1
step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

[ "$(id -u)" -ne 0 ] || { echo "Run this as your normal user, not root."; exit 1; }
grep -q '^ID=fedora' /etc/os-release || { echo "This setup is for Fedora."; exit 1; }

step "Installing COSMIC (if missing), build tools and fonts"
rpm -q cosmic-comp >/dev/null 2>&1 || sudo dnf install -y @cosmic-desktop
sudo dnf install -y git python3 rsync file flatpak libnotify gtk-update-icon-cache \
    cargo rust dnf-plugins-core rsms-inter-fonts
sudo dnf builddep -y cosmic-comp cosmic-applets cosmic-panel cosmic-workspaces

step "Installing Mac icon and cursor themes"
mkdir -p ~/.local/share/icons
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
if [ ! -d ~/.local/share/icons/MacTahoe ]; then
    git clone -q --depth 1 https://github.com/vinceliuice/MacTahoe-icon-theme "$T/mactahoe"
    (cd "$T/mactahoe" && ./install.sh -d "$HOME/.local/share/icons")
fi
if [ ! -d ~/.local/share/icons/WhiteSur ]; then
    git clone -q --depth 1 https://github.com/vinceliuice/WhiteSur-icon-theme "$T/whitesur"
    (cd "$T/whitesur" && ./install.sh -d "$HOME/.local/share/icons")
fi
if [ ! -d ~/.local/share/icons/WhiteSur-cursors ]; then
    git clone -q --depth 1 https://github.com/vinceliuice/WhiteSur-cursors "$T/cursors"
    (cd "$T/cursors" && ./install.sh)
fi

step "Installing top bar applets (Flatpak)"
flatpak remote-add --user --if-not-exists cosmic https://apt.pop-os.org/cosmic/cosmic.flatpakrepo
flatpak install --user -y --noninteractive cosmic \
    io.github.cosmic_utils.minimon-applet \
    io.github.cosmic_utils.cosmic-ext-applet-clipboard-manager \
    com.github.codevardhan.caffeine-applet \
    io.github.marcelogomes90.cosmic-ext-applet-status-hub || echo "Some applets failed to install; the rest still works."

step "Backing up your current COSMIC settings"
if [ -d ~/.config/cosmic ]; then
    B=~/.config/cosmic.backup-$(date +%Y%m%d-%H%M%S)
    cp -a ~/.config/cosmic "$B" && echo "Saved to $B"
fi

step "Copying the customization toolkit"
mkdir -p ~/.local/bin ~/.local/share/applications ~/.config/systemd/user ~/.config/cosmic-mac
rsync -a --exclude '*built-version' --exclude rebuild.log --exclude .lock \
    "$S/.local/share/cosmic-mac/" "$D/"
for f in "$S"/.local/bin/cosmic-*; do install -m755 "$f" ~/.local/bin/; done
cp "$S"/.local/share/applications/com.aqib.*.desktop ~/.local/share/applications/
cp "$S"/.config/systemd/user/cosmic-mac-* ~/.config/systemd/user/
[ -f ~/.config/cosmic-mac/animation.conf ] || cp "$S/.config/cosmic-mac/animation.conf" ~/.config/cosmic-mac/

if [ -n "$RESTORE" ]; then
    step "Restoring all saved COSMIC settings"
    rsync -a "$S/.config/cosmic/" ~/.config/cosmic/
fi

step "Building the patched COSMIC parts and helper apps (takes a while)"
"$D/rebuild.sh" --force || echo "Some parts failed to build; see $D/rebuild.log. Everything else is installed."

step "Enabling automatic rebuilds after COSMIC updates"
systemctl --user daemon-reload
systemctl --user enable --now cosmic-mac-rebuild.timer cosmic-mac-panel-contrast.path

"$R/check.sh" || true
printf '\nDone. Log out, pick COSMIC on the login screen, and log back in.\n'
