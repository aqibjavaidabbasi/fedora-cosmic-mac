COSMIC Mac customizations
=========================
apply-settings.sh       Re-applies all settings (dock, glass theme, icons, font, swipes, shortcuts...).
                        Edit it to change what gets re-applied.
rebuild.sh              Rebuilds the three-finger-swipe compositor + maximize helper for the
                        installed COSMIC version. Add --force to rebuild anyway. Log: rebuild.log
cosmic-comp-launcher.sh Installed as ~/.local/bin/cosmic-comp. Uses the patched build only when it
                        matches the installed COSMIC; otherwise stock COSMIC + background rebuild.
maximize-helper/        Source of the "maximized app gets its own workspace" program.
panel-contrast.py       Sets the top bar to light or dark text to match the wallpaper; runs automatically
                        when the wallpaper changes (cosmic-mac-panel-contrast.path).
A daily systemd user timer (cosmic-mac-rebuild.timer) runs rebuild.sh.

Undo everything:
  systemctl --user disable --now cosmic-mac-rebuild.timer
  rm ~/.local/bin/cosmic-comp ~/.local/bin/cosmic-maximize-to-workspace ~/.config/autostart/cosmic-maximize-to-workspace.desktop

Top bar (added 2026-09-29)
  control-center/         Source of the Control Center applet (Wi-Fi, Bluetooth, Do Not Disturb,
                          brightness, volume, battery, power). Installed as
                          ~/.local/bin/cosmic-mac-control-center plus
                          ~/.local/share/applications/com.aqib.CosmicMacControlCenter.desktop.
                          Standalone: it uses command-line tools, so COSMIC updates don't need a rebuild.
  battery/                Source of the macOS-style battery item ("54%" + glyph, yellow in Low Power
                          Mode) and its Battery popover. Talks to UPower + tuned-ppd over D-Bus.
                          Build: same flags as rebuild.sh; not installed yet.
  NOTE: each display has its own bar (Panel-eDP-1, Panel-DP-1), copied from the "Panel"
  template. Settings → Panel edits only the template, so after changing applets there run:
    for o in eDP-1 DP-1; do cp ~/.config/cosmic/com.system76.CosmicPanel.Panel/v1/plugins_wings ~/.config/cosmic/com.system76.CosmicPanel.Panel-$o/v1/; done
  and keep the list in apply-settings.sh in step, or a re-run will put the old list back.
  TOPBAR-PLAN.md         Plan for the rest of the macOS top bar (Wi-Fi popover, widget drawer, theme).
  patches/notifications.py  Turns COSMIC's notifications applet into a clock + calendar +
                          notifications "Notification Center" (replaces the clock applet).
  patches/panel.py        Top bar recolors without restarting its applets.
  patches/workspaces.py   Overview: workspace thumbnails centered.
  Community applets (flatpak, remote "cosmic"): minimon, clipboard-manager, caffeine, status-hub.
