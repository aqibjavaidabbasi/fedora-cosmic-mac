#!/bin/sh
# Installed as ~/.local/bin/cosmic-settings. Uses the Mac System Settings build
# only if it matches the installed cosmic-settings version; otherwise Fedora's own.
D="$HOME/.local/share/cosmic-mac"
want=$(rpm -q --qf '%{VERSION}' cosmic-settings 2>/dev/null)
have=$(cat "$D/cosmic-settings-built-version" 2>/dev/null)
if [ -n "$want" ] && [ "$want" = "$have" ] && [ -x "$D/bin/cosmic-settings" ]; then
    exec "$D/bin/cosmic-settings" "$@"
fi
exec /usr/bin/cosmic-settings "$@"
