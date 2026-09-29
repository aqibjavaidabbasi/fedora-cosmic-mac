#!/bin/sh
# Installed as ~/.local/bin/cosmic-workspaces. Uses the centered-thumbnails build
# only if it matches the installed cosmic-workspaces version; otherwise Fedora's own.
D="$HOME/.local/share/cosmic-mac"
want=$(rpm -q --qf '%{VERSION}' cosmic-workspaces 2>/dev/null)
have=$(cat "$D/cosmic-workspaces-built-version" 2>/dev/null)
if [ -n "$want" ] && [ "$want" = "$have" ] && [ -x "$D/bin/cosmic-workspaces" ]; then
    exec "$D/bin/cosmic-workspaces" "$@"
fi
exec /usr/bin/cosmic-workspaces "$@"
