#!/bin/sh
# Installed as ~/.local/bin/cosmic-term. Uses the Mac title bar build
# only if it matches the installed cosmic-term version; otherwise Fedora's own.
D="$HOME/.local/share/cosmic-mac"
want=$(rpm -q --qf '%{VERSION}' cosmic-term 2>/dev/null)
have=$(cat "$D/cosmic-term-built-version" 2>/dev/null)
if [ -n "$want" ] && [ "$want" = "$have" ] && [ -x "$D/bin/cosmic-term" ]; then
    exec "$D/bin/cosmic-term" "$@"
fi
exec /usr/bin/cosmic-term "$@"
