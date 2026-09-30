#!/bin/sh
# Installed as ~/.local/bin/cosmic-osd. Uses the Mac alerts build
# only if it matches the installed cosmic-osd version; otherwise Fedora's own.
D="$HOME/.local/share/cosmic-mac"
want=$(rpm -q --qf '%{VERSION}' cosmic-osd 2>/dev/null)
have=$(cat "$D/cosmic-osd-built-version" 2>/dev/null)
if [ -n "$want" ] && [ "$want" = "$have" ] && [ -x "$D/bin/cosmic-osd" ]; then
    exec "$D/bin/cosmic-osd" "$@"
fi
exec /usr/bin/cosmic-osd "$@"
