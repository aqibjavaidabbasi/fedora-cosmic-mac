#!/bin/sh
# Installed as ~/.local/bin/cosmic-store. Uses the Mac App Store build
# only if it matches the installed cosmic-store version; otherwise Fedora's own.
D="$HOME/.local/share/cosmic-mac"
want=$(rpm -q --qf '%{VERSION}' cosmic-store 2>/dev/null)
have=$(cat "$D/cosmic-store-built-version" 2>/dev/null)
if [ -n "$want" ] && [ "$want" = "$have" ] && [ -x "$D/bin/cosmic-store" ]; then
    exec "$D/bin/cosmic-store" "$@"
fi
exec /usr/bin/cosmic-store "$@"
