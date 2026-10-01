#!/bin/sh
# Installed as ~/.local/bin/cosmic-player. Uses the Mac title bar build
# only if it matches the installed cosmic-player version; otherwise Fedora's own.
D="$HOME/.local/share/cosmic-mac"
want=$(rpm -q --qf '%{VERSION}' cosmic-player 2>/dev/null)
have=$(cat "$D/cosmic-player-built-version" 2>/dev/null)
if [ -n "$want" ] && [ "$want" = "$have" ] && [ -x "$D/bin/cosmic-player" ]; then
    exec "$D/bin/cosmic-player" "$@"
fi
exec /usr/bin/cosmic-player "$@"
