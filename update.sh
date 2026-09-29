#!/bin/bash
# Safe way to update Fedora and COSMIC without losing the customizations:
#   1. saves the current setup to git (and pushes it, if a remote is set)
#   2. updates Fedora packages and Flatpaks
#   3. rebuilds the patched COSMIC parts for the new versions
#   4. saves the refreshed patches to git and shows what is active
set -uo pipefail
R=$(cd "$(dirname "$0")" && pwd)
D=$HOME/.local/share/cosmic-mac
step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
save() {
    "$R/sync.sh" >/dev/null
    git -C "$R" add -A
    git -C "$R" diff --cached --quiet || git -C "$R" commit -qm "$1"
    git -C "$R" remote | grep -q . && { git -C "$R" push -q || echo "Push failed; commits are saved locally."; }
}

step "Backing up the current setup"
save "Backup before update $(date +%F)" || { echo "Backup failed; not updating."; exit 1; }

step "Updating Fedora"
sudo dnf upgrade --refresh || { echo "Fedora update failed; nothing else changed."; exit 1; }
flatpak update -y --noninteractive || true

step "Rebuilding customizations for the new COSMIC (can take 25+ minutes)"
"$D/rebuild.sh"

step "Saving the refreshed setup"
save "Update to COSMIC $(rpm -q --qf '%{VERSION}' cosmic-comp) on Fedora $(. /etc/os-release; echo "$VERSION_ID")"

step "Status"
"$R/check.sh" && printf '\nAll customizations are active. Log out and back in if anything was rebuilt.\n'
