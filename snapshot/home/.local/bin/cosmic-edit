#!/bin/sh
# Installed as ~/.local/bin/cosmic-edit. Uses the Mac title bar build
# only if it matches the installed cosmic-edit version; otherwise Fedora's own.
D="$HOME/.local/share/cosmic-mac"
want=$(rpm -q --qf '%{VERSION}' cosmic-edit 2>/dev/null)
have=$(cat "$D/cosmic-edit-built-version" 2>/dev/null)
if [ -n "$want" ] && [ "$want" = "$have" ] && [ -x "$D/bin/cosmic-edit" ]; then
    exec "$D/bin/cosmic-edit" "$@"
fi
exec /usr/bin/cosmic-edit "$@"
