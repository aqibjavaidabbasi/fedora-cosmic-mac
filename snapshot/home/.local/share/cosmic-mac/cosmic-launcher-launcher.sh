#!/bin/sh
# Installed as ~/.local/bin/cosmic-launcher. Uses the Spotlight build
# only if it matches the installed cosmic-launcher version; otherwise Fedora's own.
D="$HOME/.local/share/cosmic-mac"
want=$(rpm -q --qf '%{VERSION}' cosmic-launcher 2>/dev/null)
have=$(cat "$D/cosmic-launcher-built-version" 2>/dev/null)
if [ -n "$want" ] && [ "$want" = "$have" ] && [ -x "$D/bin/cosmic-launcher" ]; then
    exec "$D/bin/cosmic-launcher" "$@"
fi
exec /usr/bin/cosmic-launcher "$@"
