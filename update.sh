#!/bin/bash
# Safe way to update Fedora and COSMIC without losing the customizations:
#   1. updates Fedora packages and Flatpaks
#   2. rebuilds the patched COSMIC parts for the new versions
#   3. shows what is active
# Git is intentionally left untouched; use sync.sh explicitly when wanted.
set -uo pipefail
R=$(cd "$(dirname "$0")" && pwd)
D=$HOME/.local/share/cosmic-mac
step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

step "Updating Fedora"
sudo dnf upgrade --refresh || { echo "Fedora update failed; nothing else changed."; exit 1; }
flatpak update -y --noninteractive || true

step "Rebuilding customizations for the new COSMIC (can take 25+ minutes)"
"$D/rebuild.sh"

step "Status"
"$R/check.sh" && printf '\nAll customizations are active. Log out and back in if anything was rebuilt.\n'
