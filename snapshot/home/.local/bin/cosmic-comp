#!/bin/sh
# Installed as ~/.local/bin/cosmic-comp. COSMIC starts this instead of /usr/bin/cosmic-comp.
# Runs the three-finger-swipe build only if it matches the installed COSMIC version;
# otherwise runs Fedora's stock compositor (so an update never breaks the session)
# and starts a background rebuild.
D="$HOME/.local/share/cosmic-mac"
want=$(rpm -q --qf '%{VERSION}' cosmic-comp 2>/dev/null)
have=$(cat "$D/built-version" 2>/dev/null)

# Mac-like animation settings (see ~/.config/cosmic-mac/animation.conf)
if [ -f "$HOME/.config/cosmic-mac/animation.conf" ]; then
    set -a; . "$HOME/.config/cosmic-mac/animation.conf"; set +a
fi

if [ -n "$want" ] && [ "$want" = "$have" ] && [ -x "$D/bin/cosmic-comp" ]; then
    exec "$D/bin/cosmic-comp" "$@"
fi

systemd-run --user --quiet --collect --unit=cosmic-mac-rebuild-login "$D/rebuild.sh" >/dev/null 2>&1 || true
exec /usr/bin/cosmic-comp "$@"
