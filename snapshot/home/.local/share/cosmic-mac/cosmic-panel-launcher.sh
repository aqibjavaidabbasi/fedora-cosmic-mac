#!/bin/sh
# Installed as ~/.local/bin/cosmic-panel. Uses the recolor-without-restart build
# only if it matches the installed cosmic-panel version; otherwise Fedora's own.
D="$HOME/.local/share/cosmic-mac"
want=$(rpm -q --qf '%{VERSION}' cosmic-panel 2>/dev/null)
have=$(cat "$D/cosmic-panel-built-version" 2>/dev/null)
if [ -n "$want" ] && [ "$want" = "$have" ] && [ -x "$D/bin/cosmic-panel" ]; then
    exec "$D/bin/cosmic-panel" "$@"
fi
exec /usr/bin/cosmic-panel "$@"
