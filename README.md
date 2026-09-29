# cosmic-mac

Snapshot of my COSMIC desktop setup on Fedora (Surface kernel), taken 2026-09-29 before further macOS-style customization.

`snapshot/home/` mirrors `$HOME`:

- `.config/cosmic/` – all COSMIC settings (panel, dock, theme, shortcuts, workspaces, applets)
- `.config/cosmic-mac/` – swipe/animation tuning read by the patched compositor
- `.config/systemd/user/cosmic-mac-*` – rebuild timer and panel-contrast watcher
- `.config/autostart/` – maximize-to-workspace helper entry
- `.local/state/cosmic*` – COSMIC state (outputs, a11y)
- `.local/share/cosmic-mac/` – patch scripts, launchers, Rust sources for the helper apps, rebuild script
- `.local/bin/cosmic-*` – launcher wrappers that swap in the patched COSMIC components
- `.bash_profile` – session env (Mac cursor)

Not included: compiled binaries (`.local/share/cosmic-mac/bin`, ELF files in `.local/bin`) and the upstream source checkouts in `.local/share/cosmic-mac/build` – `rebuild.sh` recreates them from `patches/`.

Restore: copy `snapshot/home/` over `$HOME`, then log out and back in.
