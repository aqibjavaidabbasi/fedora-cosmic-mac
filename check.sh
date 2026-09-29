#!/bin/bash
# Shows whether every customization is active or temporarily on stock COSMIC.
D=$HOME/.local/share/cosmic-mac
bad=0
ok()   { printf '  \033[32m✓\033[0m %s\n' "$1"; }
warn() { printf '  \033[33m✗\033[0m %s\n' "$1"; bad=1; }

echo "Patched COSMIC parts (need to match the installed COSMIC version):"
while read -r rpm stamp bin label; do
    want=$(rpm -q --qf '%{VERSION}' "$rpm" 2>/dev/null)
    have=$(cat "$D/$stamp" 2>/dev/null)
    if [ -n "$want" ] && [ "$want" = "$have" ] && [ -x "$D/bin/$bin" ]; then ok "$label ($want)"
    else warn "$label: built for ${have:-nothing}, installed ${want:-none} -> run $D/rebuild.sh"; fi
done <<'EOF'
cosmic-comp built-version cosmic-comp Three-finger swipe, Mac swipe feel, Mac Spaces
cosmic-applets applets-built-version cosmic-panel-button Top-left bar buttons
cosmic-applets cosmic-applet-notifications-built-version cosmic-applet-notifications Notification Center
cosmic-panel cosmic-panel-built-version cosmic-panel Top bar recolor
cosmic-workspaces cosmic-workspaces-built-version cosmic-workspaces Centered overview thumbnails
EOF

echo "Helper apps:"
for b in cosmic-mac-control-center cosmic-mac-battery cosmic-mac-wifi cosmic-maximize-to-workspace; do
    f=$HOME/.local/bin/$b
    if [ ! -x "$f" ]; then warn "$b missing -> run $D/rebuild.sh"
    elif ldd "$f" 2>/dev/null | grep -q "not found"; then warn "$b needs a rebuild -> run $D/rebuild.sh --force"
    else ok "$b"; fi
done
[ -x "$D/bin/cosmic-mac-theme" ] && ok "glass theme tool" || warn "glass theme tool missing -> run $D/rebuild.sh --force"

echo "Session:"
p=$(env -i HOME="$HOME" SHELL="$SHELL" USER="$USER" "$SHELL" -lc 'command -v cosmic-comp' 2>/dev/null)
[ "$p" = "$HOME/.local/bin/cosmic-comp" ] && ok "login session uses the patched launchers" \
    || warn "login session finds cosmic-comp at '$p' -> run $D/apply-settings.sh"
for u in cosmic-mac-rebuild.timer cosmic-mac-panel-contrast.path; do
    systemctl --user is-enabled -q "$u" 2>/dev/null && ok "$u enabled" || warn "$u not enabled -> systemctl --user enable --now $u"
done
exit $bad
