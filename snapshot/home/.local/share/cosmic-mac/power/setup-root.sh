#!/bin/bash
# Battery-saving system changes that need root. Run once:
#   sudo bash ~/.local/share/cosmic-mac/power/setup-root.sh
# Undo everything with:
#   sudo bash ~/.local/share/cosmic-mac/power/undo-root.sh
#
#  1. Developer menu: a polkit rule so you can start/stop/restart exactly the system units
#     listed in ~/.config/cosmic-mac/dev-services without a password (active local session,
#     you or the wheel group). Run this script again after adding services to that file.
#  2. Those services no longer start at boot (they keep running now, and start from the
#     Developer menu in the top bar whenever you need them).
#  3. Closing the lid suspends (on battery and on power); with an external display
#     connected (docked) closing it does nothing. Overrides the HandleLidSwitch=lock lines
#     of /etc/systemd/logind.conf.d/surface-freeze-fix.conf with a later drop-in (that file
#     itself is left as it is). logind re-reads its settings without restarting.
#
# Everything changed is recorded in /var/lib/cosmic-mac-power so undo-root.sh can put it back.
set -euo pipefail
[ "$(id -u)" -eq 0 ] || { echo "Run with sudo: sudo bash $0"; exit 1; }

U=${SUDO_USER:-aqibj}
H=$(getent passwd "$U" | cut -d: -f6)
B=/var/lib/cosmic-mac-power
RULE=/etc/polkit-1/rules.d/49-cosmic-mac-dev-services.rules
LID=/etc/systemd/logind.conf.d/zz-cosmic-mac-lid-suspend.conf
mkdir -p "$B"
step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

# System units from the Developer menu's list ("--user" lines are user units: no root needed)
CONF=$H/.config/cosmic-mac/dev-services
if [ -f "$CONF" ]; then
    mapfile -t UNITS < <(grep -v '^\s*#' "$CONF" | grep -v '^\s*--user' | awk 'NF{u=$1; if (u !~ /\./) u=u".service"; print u}')
else
    UNITS=(mariadb.service postgresql.service nginx.service php-fpm.service supervisord.service)
fi
echo "Developer services (system): ${UNITS[*]}"

step "Polkit rule: start/stop the developer services without a password"
[ -f "$RULE" ] && [ ! -f "$B/polkit-rule.orig" ] && cp -a "$RULE" "$B/polkit-rule.orig"
LIST=$(printf '"%s", ' "${UNITS[@]}"); LIST=${LIST%, }
cat > "$RULE.tmp" <<EOF
// cosmic-mac: the top bar's Developer menu may start, stop and restart these units
// without a password (user $U or the wheel group, in an active local session).
// Written by ~/.local/share/cosmic-mac/power/setup-root.sh; removed by undo-root.sh.
polkit.addRule(function (action, subject) {
    if (action.id != "org.freedesktop.systemd1.manage-units") return polkit.Result.NOT_HANDLED;
    if (!subject.local || !subject.active) return polkit.Result.NOT_HANDLED;
    if (subject.user != "$U" && !subject.isInGroup("wheel")) return polkit.Result.NOT_HANDLED;
    var units = [$LIST];
    var verbs = ["start", "stop", "restart"];
    if (units.indexOf(action.lookup("unit")) >= 0 && verbs.indexOf(action.lookup("verb")) >= 0)
        return polkit.Result.YES;
    return polkit.Result.NOT_HANDLED;
});
EOF
chmod 644 "$RULE.tmp"
mv -f "$RULE.tmp" "$RULE"
echo "Wrote $RULE"

step "Developer services: don't start at boot (left running now)"
touch "$B/enabled-units.txt"
for u in "${UNITS[@]}"; do
    if systemctl is-enabled -q "$u" 2>/dev/null; then
        grep -qx "$u" "$B/enabled-units.txt" || echo "$u" >> "$B/enabled-units.txt"
        systemctl disable -q "$u" && echo "disabled $u (still $(systemctl is-active "$u"))"
    else
        echo "$u: already not started at boot"
    fi
done

step "Lid: close to suspend (ignored while docked to an external display)"
for f in /etc/systemd/logind.conf.d/*.conf; do
    [ -f "$f" ] && [ "$f" != "$LID" ] && cp -an "$f" "$B/" && grep -H 'HandleLid' "$f" || true
done
mkdir -p "$(dirname "$LID")"
cat > "$LID" <<'EOF'
# cosmic-mac: closing the lid suspends, to save battery. Sorts after (and so overrides)
# the HandleLidSwitch=lock lines in surface-freeze-fix.conf. With an external display
# connected, closing the lid does nothing. Removed by power/undo-root.sh.
[Login]
HandleLidSwitch=suspend
HandleLidSwitchExternalPower=suspend
HandleLidSwitchDocked=ignore
EOF
chmod 644 "$LID"
echo "Wrote $LID"
# logind re-reads its configuration on SIGHUP; restarting it would end the session.
systemctl kill -s HUP systemd-logind
sleep 1
echo "logind now: $(busctl get-property org.freedesktop.login1 /org/freedesktop/login1 org.freedesktop.login1.Manager HandleLidSwitch 2>/dev/null)"

printf '\nDone. Undo with: sudo bash %s/undo-root.sh\n' "$(dirname "$(readlink -f "$0")")"
