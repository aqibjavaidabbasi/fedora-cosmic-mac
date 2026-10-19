#!/bin/sh
# Keeps the window buttons on the left, like macOS. COSMIC's settings daemon resets the GNOME
# "button-layout" (what Firefox and other GTK apps read) to the right side at every login,
# so this sets it again and again whenever it changes. Run by cosmic-mac-buttons.service.
LAYOUT="close,minimize,maximize:"
KEY="org.gnome.desktop.wm.preferences"
fix() {
    [ "$(gsettings get $KEY button-layout)" = "'$LAYOUT'" ] || gsettings set $KEY button-layout "$LAYOUT"
}
fix
gsettings monitor $KEY button-layout | while read -r _; do fix; done
