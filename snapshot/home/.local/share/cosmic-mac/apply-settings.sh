#!/bin/bash
# Re-applies all macOS-like COSMIC customizations. Safe to run any time.
# Run by install.sh, and by you after editing it. Rebuilds after updates do NOT run it,
# so changes you make in COSMIC Settings are kept.

C=~/.config/cosmic
put() { mkdir -p "$(dirname "$1")"; printf '%s' "$2" > "$1"; }

# --- Workspaces: separate per display, left/right -------------------------
put $C/com.system76.CosmicComp/v1/workspaces '(
    workspace_mode: OutputBound,
    workspace_layout: Horizontal,
    workspace_wraparound: false,
)'

# --- Touchpad: no tap-to-click, natural scroll, pause while typing ---------
put $C/com.system76.CosmicComp/v1/input_touchpad '(state:Enabled,click_method:Some(Clickfinger),disable_while_typing:Some(true),scroll_config:Some((method:Some(TwoFinger),natural_scroll:Some(true),scroll_button:None,scroll_factor:None)),tap_config:Some((enabled:false,button_map:Some(LeftRightMiddle),drag:true,drag_lock:false)))'

# --- Dock: floating glass, hides when a window covers it, apps only --------
D=$C/com.system76.CosmicPanel.Dock/v1
put $D/autohide OnOverlap
put $D/exclusive_zone false
put $D/keep_style_on_maximize true
put $D/opacity 0.5
put $D/plugins_center 'Some([
    "com.system76.CosmicAppList",
    "com.system76.CosmicAppletMinimize",
])'

# --- Top bar: Mac layout, clock on the right, nearly transparent -----------
P=$C/com.system76.CosmicPanel.Panel/v1
put $P/opacity 0.2
put $P/plugins_center 'Some([])'
put $P/plugins_wings 'Some(([
    "com.system76.CosmicPanelAppButton",
    "com.system76.CosmicPanelLauncherButton",
    "com.system76.CosmicPanelWorkspacesButton",
], [
    "com.system76.CosmicAppletPower",
    "com.system76.CosmicAppletBluetooth",
    "com.aqib.CosmicMacBattery",
    "io.github.cosmic_utils.minimon-applet",
    "io.github.cosmic_utils.cosmic-ext-applet-clipboard-manager",
    "com.github.codevardhan.caffeine-applet",
    "io.github.marcelogomes90.cosmic-ext-applet-status-hub",
    "com.system76.CosmicAppletBattery",
    "com.aqib.CosmicMacControlCenter",
    "com.system76.CosmicAppletNotifications",
]))'
put $C/com.system76.CosmicAppletTime/v1/show_date_in_top_panel true
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
put $C/com.system76.CosmicPanelButton/v1/configs '{
    "*/com.system76.CosmicAppLibrary": (force_presentation: Some(Icon)),
    "Dock": (force_presentation: Some(Icon)),
}'

# --- Icons, font, compact title bars ---------------------------------------
K=$C/com.system76.CosmicTk/v1
put $K/icon_theme '"MacTahoe-COSMIC"'
put $K/header_size Compact
put $K/interface_font '(
    family: "Inter",
    weight: Normal,
    stretch: Normal,
    style: Normal,
)'

# --- Super+Space opens app search (like Cmd+Space) -------------------------
put $C/com.system76.CosmicSettings.Shortcuts/v1/custom '{
    (modifiers: [Super], key: "space"): System(Launcher),
}'

# --- GTK apps inside COSMIC (COSMIC has its own settings database) ---------
DCONF_PROFILE=cosmic gsettings set org.gnome.desktop.interface cursor-theme 'WhiteSur-cursors'
DCONF_PROFILE=cosmic gsettings set org.gnome.desktop.interface icon-theme 'MacTahoe-COSMIC'
DCONF_PROFILE=cosmic gsettings set org.gnome.desktop.interface font-name 'Inter 11'

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

echo "COSMIC customizations applied."
