#!/bin/sh
# Installed as ~/.local/bin/cosmic-app-library. Uses the Launchpad build
# only if it matches the installed cosmic-app-library version; otherwise Fedora's own.
D="$HOME/.local/share/cosmic-mac"
want=$(rpm -q --qf '%{VERSION}' cosmic-app-library 2>/dev/null)
have=$(cat "$D/cosmic-app-library-built-version" 2>/dev/null)
if [ -n "$want" ] && [ "$want" = "$have" ] && [ -x "$D/bin/cosmic-app-library" ]; then
    exec "$D/bin/cosmic-app-library" "$@"
fi
exec /usr/bin/cosmic-app-library "$@"
