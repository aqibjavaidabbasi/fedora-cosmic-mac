#!/bin/sh
# Installed as ~/.local/bin/cosmic-applet-notifications. Uses the Notification
# Center build (clock + calendar + notifications) only if it matches the installed
# cosmic-applets version; otherwise Fedora's own.
D="$HOME/.local/share/cosmic-mac"
want=$(rpm -q --qf '%{VERSION}' cosmic-applets 2>/dev/null)
have=$(cat "$D/cosmic-applet-notifications-built-version" 2>/dev/null)
if [ -n "$want" ] && [ "$want" = "$have" ] && [ -x "$D/bin/cosmic-applet-notifications" ]; then
    exec "$D/bin/cosmic-applet-notifications" "$@"
fi
exec /usr/bin/cosmic-applet-notifications "$@"
