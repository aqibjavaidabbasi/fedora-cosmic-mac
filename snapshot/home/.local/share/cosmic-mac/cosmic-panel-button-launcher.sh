#!/bin/sh
# Installed as ~/.local/bin/cosmic-panel-button. Uses the per-button icon/text build
# only if it matches the installed cosmic-applets version; otherwise Fedora's own.
D="$HOME/.local/share/cosmic-mac"
want=$(rpm -q --qf '%{VERSION}' cosmic-applets 2>/dev/null)
have=$(cat "$D/applets-built-version" 2>/dev/null)
if [ -n "$want" ] && [ "$want" = "$have" ] && [ -x "$D/bin/cosmic-panel-button" ]; then
    exec "$D/bin/cosmic-panel-button" "$@"
fi
exec /usr/bin/cosmic-panel-button "$@"
