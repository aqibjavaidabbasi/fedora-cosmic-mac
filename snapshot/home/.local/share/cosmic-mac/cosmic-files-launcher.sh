#!/bin/sh
# Installed as ~/.local/bin/cosmic-files. Uses the Finder build
# only if it matches the installed cosmic-files version; otherwise Fedora's own.
D="$HOME/.local/share/cosmic-mac"
want=$(rpm -q --qf '%{VERSION}' cosmic-files 2>/dev/null)
have=$(cat "$D/cosmic-files-built-version" 2>/dev/null)
if [ -n "$want" ] && [ "$want" = "$have" ] && [ -x "$D/bin/cosmic-files" ]; then
    exec "$D/bin/cosmic-files" "$@"
fi
exec /usr/bin/cosmic-files "$@"
