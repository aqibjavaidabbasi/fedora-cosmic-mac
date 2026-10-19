#!/bin/bash
# Gives the Claude desktop app the system (Mac-style) title bar.
#
# The app (the Fedora "claude-desktop-unofficial" package, built on Anthropic's official
# Linux build) opens its main window with its own hidden title bar and draws its own window
# buttons, so COSMIC's traffic lights can't apply. This makes a patched copy of the app in
# ~/.local/share/claude-desktop-mac (a reflink copy: instant, takes no extra disk space) in
# which that one window option is changed to the normal system title bar, plus a launcher
# ~/.local/bin/claude-desktop-mac that starts the copy and re-creates it when the package
# updates. The system package is never modified.
#
#   claude-titlebar.sh --ensure   re-create the copy if the installed version changed
#   claude-titlebar.sh --force    re-create it now
#   claude-titlebar.sh --remove   delete the copy and the launcher (back to the stock app)
SRC=/usr/lib/claude-desktop-unofficial
DST=$HOME/.local/share/claude-desktop-mac
BIN=$HOME/.local/bin/claude-desktop-mac
APPS=$HOME/.local/share/applications
ver() { rpm -q --qf '%{VERSION}-%{RELEASE}' claude-desktop-unofficial 2>/dev/null; }

remove() {
    rm -rf "$DST" "$BIN" "$APPS/claude-desktop-unofficial.desktop"
    echo "Removed the patched Claude copy; the stock app is used again."
}

build() {
    [ -d "$SRC" ] || { echo "Claude Desktop is not installed."; return 1; }
    rm -rf "$DST.new"
    cp -a --reflink=auto "$SRC" "$DST.new" || return 1
    python3 - "$DST.new/resources/app.asar" <<'PY' || { rm -rf "$DST.new"; return 1; }
import re, sys
p = sys.argv[1]
data = open(p, "rb").read()
# main window: hidden title bar + overlay buttons  ->  the normal system title bar
# (same length, padded with spaces, so the asar's file offsets stay valid)
pat = re.compile(rb'titleBarStyle:"hidden",titleBarOverlay:!0,')
hits = pat.findall(data)
if len(hits) != 1:
    sys.exit("Claude's window code changed (%d matches); not patching." % len(hits))
new = b'titleBarStyle:"default",'
new = new + b" " * (len(hits[0]) - len(new))
open(p, "wb").write(pat.sub(lambda m: new, data, count=1))
PY
    # launcher: the stock one, pointed at the patched copy
    mkdir -p "$(dirname "$BIN")" "$APPS"
    sed -e "s|^app_exec=.*|app_exec=\"$DST/claude-desktop\"|" \
        -e "s|^app_dir=.*|app_dir=\"$DST\"|" \
        -e '/^source "\/usr\/lib\/claude-desktop-unofficial\/launcher-common.sh"/a \
# cosmic-mac: re-create the patched copy first if the installed version changed\
"$HOME/.local/share/cosmic-mac/claude-titlebar.sh" --ensure >/dev/null 2>\&1 || true' \
        /usr/bin/claude-desktop-unofficial > "$BIN.tmp"
    chmod +x "$BIN.tmp"
    mv -f "$BIN.tmp" "$BIN"   # replace atomically: the launcher may be running
    # menu / Dock entry: same name as the system one, so it replaces it for this user
    sed -e "s|^Exec=.*|Exec=$BIN %u|" /usr/share/applications/claude-desktop-unofficial.desktop > "$APPS/claude-desktop-unofficial.desktop" 2>/dev/null \
        || sed -e "s|^Exec=.*|Exec=$BIN %u|" "$(ls /usr/share/applications/*laude*.desktop | head -1)" > "$APPS/claude-desktop-unofficial.desktop"
    echo "$(ver)" > "$DST.new/.cosmic-mac-version"
    rm -rf "$DST"
    mv "$DST.new" "$DST"
    echo "Patched Claude ready ($(ver))."
}

case "${1:---ensure}" in
    --remove) remove ;;
    --force) build ;;
    *)
        [ "$(cat "$DST/.cosmic-mac-version" 2>/dev/null)" = "$(ver)" ] && [ -x "$BIN" ] && exit 0
        build ;;
esac
