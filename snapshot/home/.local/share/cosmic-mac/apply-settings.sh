#!/bin/bash
# Re-applies all macOS-like COSMIC customizations. Safe to run any time.
# Run by install.sh, and by you after editing it. Rebuilds after updates do NOT run it,
# so changes you make in COSMIC Settings are kept.

C=~/.config/cosmic
put() { mkdir -p "$(dirname "$1")"; printf '%s' "$2" > "$1"; }

# --- Fonts: Apple's SF Pro when installed (install-sf-pro.sh), otherwise Inter ---
if fc-list : family | grep -qx "SF Pro Text"; then UI_FONT="SF Pro Text"; else UI_FONT="Inter"; fi
K=$C/com.system76.CosmicTk/v1
put $K/interface_font "(
    family: \"$UI_FONT\",
    weight: Normal,
    stretch: Normal,
    style: Normal,
)"
DCONF_PROFILE=cosmic gsettings set org.gnome.desktop.interface font-name "$UI_FONT 11"
DCONF_PROFILE=cosmic gsettings set org.gnome.desktop.interface document-font-name "$UI_FONT 11"
DCONF_PROFILE=cosmic gsettings set org.gnome.desktop.wm.preferences titlebar-font "$UI_FONT Semi-Bold 11"
# System sans-serif, system-ui and the web's -apple-system / BlinkMacSystemFont -> SF Pro
mkdir -p ~/.config/fontconfig/conf.d
cp -f "$HOME/.local/share/cosmic-mac/fonts/60-cosmic-mac-sf-pro.conf" ~/.config/fontconfig/conf.d/ 2>/dev/null || true
fc-cache -f >/dev/null 2>&1 || true
[ "${1:-}" = "--fonts-only" ] && { echo "Fonts applied: $UI_FONT"; exit 0; }


# --- Workspaces: separate per display, left/right -------------------------
put $C/com.system76.CosmicComp/v1/workspaces '(
    workspace_mode: OutputBound,
    workspace_layout: Horizontal,
    workspace_wraparound: false,
)'

# --- Touchpad: no tap-to-click, natural scroll, pause while typing ---------
put $C/com.system76.CosmicComp/v1/input_touchpad '(state:Enabled,click_method:Some(Clickfinger),disable_while_typing:Some(true),scroll_config:Some((method:Some(TwoFinger),natural_scroll:Some(true),scroll_button:None,scroll_factor:None)),tap_config:Some((enabled:false,button_map:Some(LeftRightMiddle),drag:true,drag_lock:false)))'

# --- Dock: macOS shelf. Apps | Downloads, Trash on a glass shelf, hover magnification ---
# cosmic-maximize-to-workspace copies this into one dock per display ("Dock-<display>")
# and hides a display's dock only while it shows a maximized/fullscreen window.
D=$C/com.system76.CosmicPanel.Dock/v1
put $D/autohide OnOverlap
put $D/autohide_behavior '(
    wait_time: 1000,
    transition_time: 200,
    handle_size: 4,
    unhide_delay: 100,
)'
put $D/exclusive_zone false
put $D/keep_style_on_maximize true
# The dock panel is transparent: the icons draw a glass shelf along its bottom
# (patches/dock.py) so hovered icons can grow up out of it, like macOS.
put $D/opacity 0.0
put $D/border_radius 0
put $D/size 'Custom(88)'
put $D/padding 0
put $D/spacing 0
put $D/margin 6
put $D/plugins_wings 'Some(([], []))'
put $D/plugins_center 'Some([
    "com.system76.CosmicPanelAppButton",
    "com.system76.CosmicAppList",
    "com.aqib.DockSeparator",
    "com.aqib.DockDownloads",
    "com.aqib.DockTrash",
])'

# Dock: Files (Nautilus) instead of COSMIC Files; other pinned apps kept
AL=$C/com.system76.CosmicAppList/v1/favorites
[ -f "$AL" ] || cp /usr/share/cosmic/com.system76.CosmicAppList/v1/favorites "$AL" 2>/dev/null || true
[ ! -f "$AL" ] || sed -i 's/"com.system76.CosmicFiles"/"org.gnome.Nautilus"/' "$AL"

# --- Top bar: Mac layout, clock on the right, nearly transparent -----------
P=$C/com.system76.CosmicPanel.Panel/v1
put $P/opacity 0.2
put $P/size S          # 20px icons, like the macOS menu bar
put $P/spacing 0
put $P/plugins_center 'Some([])'
# macOS order: logo menu + app name on the left; app tray icons, extras, Bluetooth, battery,
# Wi-Fi, Spotlight, Control Center and the clock on the right.
put $P/plugins_wings 'Some(([
    "com.aqib.CosmicMacMenu",
], [
    "com.system76.CosmicAppletStatusArea",
    "com.aqib.CosmicMacStats",
    "com.github.codevardhan.caffeine-applet",
    "com.aqib.CosmicMacBluetooth",
    "com.aqib.CosmicMacBattery",
    "com.aqib.CosmicMacWifi",
    "com.system76.CosmicPanelLauncherButton",
    "com.aqib.CosmicMacControlCenter",
    "com.system76.CosmicAppletNotifications",
]))'
put $C/com.system76.CosmicAppletTime/v1/show_date_in_top_panel true

# Panel buttons: a magnifier for Spotlight in the top bar, and a Launchpad icon
# at the start of the Dock (kept if already set, e.g. from Edit Controls).
B=$C/com.system76.CosmicPanelButton/v1/configs
[ -s "$B" ] || put "$B" '{
}'
setbtn() { grep -qF "\"$1\"" "$B" || sed -i "0,/{/s||{\n    \"$1\": $2,|" "$B"; }
setbtn "*/com.system76.CosmicLauncher" '(force_presentation: Some(Icon), icon: Some("system-search-symbolic"))'
LP=$HOME/.local/share/cosmic-mac/icons/launchpad.svg
for d in Dock Dock-DP-1 Dock-eDP-1 Dock-HDMI-A-1; do
    setbtn "$d/com.system76.CosmicAppLibrary" "(force_presentation: Some(Icon), icon: Some(\"$LP\"))"
done
put $C/com.system76.CosmicAppletTime/v1/show_weekday true

# --- Glass theme: frosted blur, rounder corners, no active-window outline ---
# COSMIC 1.8 keeps its theme in the "v2" format and derives the see-through
# surfaces from it, so the theme is rebuilt with COSMIC's own library.
"$HOME/.local/share/cosmic-mac/bin/cosmic-mac-theme" || echo "Theme tool failed; run rebuild.sh --force"

# --- Mac-style icons for COSMIC's own apps (MacTahoe has none for them) -----
# A small add-on theme, MacTahoe-COSMIC, maps them to Mac-style icons and uses
# MacTahoe for everything else.
T=~/.local/share/icons/MacTahoe; O=~/.local/share/icons/MacTahoe-COSMIC
if [ -d "$T" ]; then
    mkdir -p "$O/apps/scalable"
    while read -r app icon; do
        src=$(find -L "$T/apps@2x/scalable" "$T/apps/scalable" -name "$icon.svg" 2>/dev/null | head -1)
        [ -n "$src" ] && ln -sf "$(readlink -f "$src")" "$O/apps/scalable/$app.svg"
    done <<'MAP'
com.system76.CosmicFiles org.gnome.Nautilus
com.system76.CosmicTerm utilities-terminal
com.system76.CosmicEdit org.gnome.TextEditor
com.system76.CosmicSettings preferences-system
com.system76.CosmicStore gnome-software
com.system76.CosmicPlayer org.gnome.Totem
com.system76.CosmicScreenshot org.gnome.Screenshot
com.system76.CosmicAppLibrary com.system76.CosmicLauncher
com.system76.CosmicWorkspaces workspace-switcher
com.system76.CosmicLauncher system-search
MAP
    put "$O/index.theme" '[Icon Theme]
Name=MacTahoe-COSMIC
Comment=MacTahoe icons plus Mac-style icons for COSMIC'"'"'s own apps
Inherits=MacTahoe,hicolor
Directories=apps/scalable

[apps/scalable]
Size=128
MinSize=8
MaxSize=512
Type=Scalable
Context=Applications'
    gtk-update-icon-cache -f -q "$O" 2>/dev/null || true
fi

# --- Top-left bar: one icon (Applications), the other buttons as text --------
# "*/<app>" keys are per-button settings understood by the patched
# cosmic-panel-button (see rebuild.sh); the stock one ignores them.
# Dock Downloads/Trash use MacTahoe's full-colour folder and bin.
PL=$HOME/.local/share/icons/MacTahoe/places/scalable
put $C/com.system76.CosmicPanelButton/v1/configs "{
    \"*/com.system76.CosmicAppLibrary\": (force_presentation: Some(Icon)),
    \"*/com.aqib.Downloads\": (force_presentation: Some(Icon), icon: Some(\"$PL/folder-download.svg\")),
    \"*/com.aqib.Trash\": (force_presentation: Some(Icon), icon: Some(\"$PL/user-trash.svg\")),
    \"Dock\": (force_presentation: Some(Icon)),
}"

# --- Icons, compact title bars ----------------------------------------------
K=$C/com.system76.CosmicTk/v1
put $K/icon_theme '"MacTahoe-COSMIC"'
put $K/header_size Compact

# --- Super+Space opens app search (like Cmd+Space) -------------------------
put $C/com.system76.CosmicSettings.Shortcuts/v1/custom '{
    (modifiers: [Super], key: "space"): System(Launcher),
}'

# --- GTK apps inside COSMIC (COSMIC has its own settings database) ---------
DCONF_PROFILE=cosmic gsettings set org.gnome.desktop.interface cursor-theme 'WhiteSur-cursors'
DCONF_PROFILE=cosmic gsettings set org.gnome.desktop.interface icon-theme 'MacTahoe-COSMIC'

# --- Mac cursor for the COSMIC session ------------------------------------
if ! grep -q "Mac-style cursor in the COSMIC session" ~/.bash_profile 2>/dev/null; then
    cat >> ~/.bash_profile <<'EOF'

# Mac-style cursor in the COSMIC session only (GNOME uses its own setting)
if [ "$XDG_CURRENT_DESKTOP" = "COSMIC" ]; then
    export XCURSOR_THEME=WhiteSur-cursors XCURSOR_SIZE=24
fi
EOF
fi

# --- Patched COSMIC parts: ~/.local/bin must come first in the session PATH ---
# The launchers in ~/.local/bin replace cosmic-comp, cosmic-panel, etc. COSMIC's
# session reads the login shell profile, so make sure it puts them first.
case "$(basename "${SHELL:-bash}")" in
    zsh) PROFILE=~/.zprofile ;;
    *)   PROFILE=~/.bash_profile ;;
esac
if ! grep -q "COSMIC Mac: patched components first" "$PROFILE" 2>/dev/null; then
    cat >> "$PROFILE" <<'EOF'

# COSMIC Mac: patched components first (launchers in ~/.local/bin)
case "$PATH" in "$HOME/.local/bin:"*) ;; *) export PATH="$HOME/.local/bin:$PATH" ;; esac
EOF
fi

# --- Maximized apps get their own workspace (autostart, COSMIC only) -------
put ~/.config/autostart/cosmic-maximize-to-workspace.desktop "[Desktop Entry]
Type=Application
Name=Maximize to Workspace
Comment=Maximized and fullscreen windows get their own workspace (macOS-like)
Exec=$HOME/.local/bin/cosmic-maximize-to-workspace
OnlyShowIn=COSMIC;
X-GNOME-Autostart-enabled=false
NoDisplay=true
"

# --- Per-display top bars ----------------------------------------------------
# cosmic-maximize-to-workspace gives every display its own bar ("Panel-<display>")
# copied from the "Panel" template above; copy template changes into them too.
for d in "$C"/com.system76.CosmicPanel.Panel-*/v1; do
    [ -d "$d" ] || continue
    for f in "$C"/com.system76.CosmicPanel.Panel/v1/*; do
        case "$(basename "$f")" in output|name|background|opacity) continue ;; esac
        cp -f "$f" "$d/"
    done
done
# Bars showing the desktop: re-tint with the wallpaper color.
"$HOME/.local/share/cosmic-mac/panel-contrast.py" || true

# Open/save dialogs from Files (Nautilus, GNOME portal) instead of COSMIC Files
mkdir -p ~/.config/xdg-desktop-portal
cat > ~/.config/xdg-desktop-portal/cosmic-portals.conf <<'PORTALS'
# cosmic-mac: open/save dialogs come from Files (Nautilus, via the GNOME portal)
# instead of COSMIC Files; everything else stays with COSMIC's portal.
[preferred]
default=cosmic;gtk;
org.freedesktop.impl.portal.FileChooser=gnome;gtk;
org.freedesktop.impl.portal.Secret=oo7-portal;gnome-keyring;
PORTALS
systemctl --user try-restart xdg-desktop-portal 2>/dev/null || true

# The COSMIC Store is called App Store
sed -e 's/^Name=COSMIC Store$/Name=App Store/' -e '/^Name\[/d' /usr/share/applications/com.system76.CosmicStore.desktop \
    > ~/.local/share/applications/com.system76.CosmicStore.desktop 2>/dev/null || true

# If COSMIC Files is still installed, keep it out of Launchpad and search
sed '/^\[Desktop Entry\]/a NoDisplay=true' /usr/share/applications/com.system76.CosmicFiles.desktop \
    > ~/.local/share/applications/com.system76.CosmicFiles.desktop 2>/dev/null || true
# Files windows get a blurred see-through sidebar (the patched compositor reads this list)
[ -f ~/.config/cosmic-mac/blur-apps ] || printf '# Apps whose windows are blurred behind, like macOS vibrancy (one app id per line)\norg.gnome.Nautilus\ncom.system76.CosmicStore\n' > ~/.config/cosmic-mac/blur-apps

# Window buttons on the left like macOS (GTK apps; the style is in gtk/mac-titlebar.css)
gsettings set org.gnome.desktop.wm.preferences button-layout 'close,minimize,maximize:' 2>/dev/null || true
for d in gtk-3.0 gtk-4.0; do
    f=~/.config/$d/settings.ini; mkdir -p ~/.config/$d; [ -f "$f" ] || printf '[Settings]\n' > "$f"
    grep -q gtk-decoration-layout "$f" || echo "gtk-decoration-layout=close,minimize,maximize:" >> "$f"
done

# Clippy (Flatpak) can only show its top-bar icon if it may own its StatusNotifier name
flatpak list --app --columns=application 2>/dev/null | grep -qx io.github._0_don.clippy \
    && flatpak override --user --own-name='org.kde.*' io.github._0_don.clippy

# --- Files (Nautilus): System Settings style sidebar (kept by cosmic-mac-gtk-css.path)
"$HOME/.local/share/cosmic-mac/gtk-css.sh"

echo "COSMIC customizations applied."
