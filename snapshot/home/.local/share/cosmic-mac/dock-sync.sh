#!/bin/bash
# Keeps the per-display docks ("Dock-<display>", made by cosmic-maximize-to-workspace)
# in step with the "Dock" settings that COSMIC Settings edits. Run by
# cosmic-mac-dock-sync.path whenever those settings or the panel list change.
C=$HOME/.config/cosmic
T=$C/com.system76.CosmicPanel.Dock/v1
E=$C/com.system76.CosmicPanel/v1/entries
[ -f "$E" ] || exit 0

python3 - "$E" <<'PY'
import os, re, sys
path = sys.argv[1]
text = open(path).read()
entries = re.findall(r'"([^"]+)"', text)
dock_on = "Dock" in entries
outputs = [e[len("Panel-"):] for e in entries if e.startswith("Panel-")]
new = [e for e in entries if dock_on or not e.startswith("Dock-")]
if dock_on:
    for o in outputs:
        if f"Dock-{o}" not in new:
            new.append(f"Dock-{o}")
if new != entries:
    open(path, "w").write("[\n" + "\n".join(f'    "{e}",' for e in new) + "\n]")
PY

for d in "$C"/com.system76.CosmicPanel.Dock-*/v1; do
    [ -d "$d" ] || continue
    for f in "$T"/*; do
        k=$(basename "$f")
        case "$k" in output|name|autohide) continue ;; esac
        cmp -s "$f" "$d/$k" || cp -f "$f" "$d/$k"
    done
done
