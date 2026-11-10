#!/bin/bash
# Reverses power/setup-root.sh:
#   sudo bash ~/.local/share/cosmic-mac/power/undo-root.sh
# Removes the polkit rule (or restores the one that was there) and the lid drop-in,
# lets logind re-read its settings, and makes the developer services start at boot again.
set -euo pipefail
[ "$(id -u)" -eq 0 ] || { echo "Run with sudo: sudo bash $0"; exit 1; }

B=/var/lib/cosmic-mac-power
RULE=/etc/polkit-1/rules.d/49-cosmic-mac-dev-services.rules
LID=/etc/systemd/logind.conf.d/zz-cosmic-mac-lid-suspend.conf

if [ -f "$B/polkit-rule.orig" ]; then
    cp -a "$B/polkit-rule.orig" "$RULE" && echo "Restored the previous $RULE"
else
    rm -f "$RULE" && echo "Removed $RULE"
fi

rm -f "$LID" && echo "Removed $LID"
systemctl kill -s HUP systemd-logind
sleep 1
echo "logind now: $(busctl get-property org.freedesktop.login1 /org/freedesktop/login1 org.freedesktop.login1.Manager HandleLidSwitch 2>/dev/null)"

if [ -f "$B/enabled-units.txt" ]; then
    while read -r u; do
        [ -n "$u" ] && systemctl enable -q "$u" && echo "re-enabled $u at boot"
    done < "$B/enabled-units.txt"
fi

rm -rf "$B"
echo "Done: system settings are back as they were."
echo "(The user unit supervisor-studio.service is yours: systemctl --user enable supervisor-studio.service)"
