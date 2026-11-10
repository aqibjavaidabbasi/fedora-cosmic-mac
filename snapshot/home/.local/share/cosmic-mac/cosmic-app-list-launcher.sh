#!/bin/sh
# Installed as ~/.local/bin/cosmic-app-list. Uses the Mac-style Dock build (patches/dock.py)
# only if it matches the installed cosmic-applets version; otherwise Fedora's own.
D="$HOME/.local/share/cosmic-mac"
want=$(rpm -q --qf '%{VERSION}' cosmic-applets 2>/dev/null)
have=$(cat "$D/cosmic-app-list-built-version" 2>/dev/null)
if [ -n "$want" ] && [ "$want" = "$have" ] && [ -x "$D/bin/cosmic-app-list" ]; then
    exec "$D/bin/cosmic-app-list" "$@"
fi
exec /usr/bin/cosmic-app-list "$@"
