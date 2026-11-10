#!/bin/sh
# Installed as ~/.local/bin/cosmic-notifications. Uses the Mac banner build
# only if it matches the installed cosmic-notifications version; otherwise Fedora's own.
D="$HOME/.local/share/cosmic-mac"
want=$(rpm -q --qf '%{VERSION}' cosmic-notifications 2>/dev/null)
have=$(cat "$D/cosmic-notifications-built-version" 2>/dev/null)
if [ -n "$want" ] && [ "$want" = "$have" ] && [ -x "$D/bin/cosmic-notifications" ]; then
    exec "$D/bin/cosmic-notifications" "$@"
fi
exec /usr/bin/cosmic-notifications "$@"
