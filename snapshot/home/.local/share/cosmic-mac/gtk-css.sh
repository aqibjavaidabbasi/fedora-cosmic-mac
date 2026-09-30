#!/bin/sh
# Keeps the macOS sidebar style for Files (Nautilus) in the GTK theme COSMIC generates.
# COSMIC rewrites ~/.config/gtk-4.0/cosmic/{dark,light}.css when the theme changes,
# so this prepends an @import of gtk/mac-sidebar-*.css again (run by
# cosmic-mac-panel-contrast.path whenever those files change, and by apply-settings.sh).
D="$HOME/.local/share/cosmic-mac/gtk"
for mode in dark light; do
    for f in "$HOME/.config/gtk-4.0/cosmic/$mode.css" "$HOME/.config/gtk-3.0/cosmic/$mode.css"; do
        [ -f "$f" ] || continue
        line="@import url(\"file://$D/mac-sidebar-$mode.css\");"
        grep -qF "$line" "$f" || sed -i "1i $line" "$f"
    done
done
