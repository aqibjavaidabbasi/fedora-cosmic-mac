# COSMIC Mac

A macOS-style COSMIC desktop for Fedora. It survives COSMIC and Fedora updates without losing features.

## Features

- **Gestures:** three-finger swipe between workspaces with a Mac-like glide. Each maximized app gets its own space, like Mission Control.
- **Top bar:** transparent Mac-style bar that recolors to match the wallpaper, with a Control Center (Wi-Fi, Bluetooth, Do Not Disturb, sound, brightness, power), a Mac battery item, and a Notification Center (clock, calendar, notifications).
- **Dock and look:** floating glass dock, frosted glass theme, MacTahoe icons, WhiteSur cursor and the Inter font.
- **Spotlight:** Super+Space opens a centered Spotlight Search pill with Apps, Files, Shortcuts and Settings buttons; suggestions appear as you type.
- **App switcher:** Alt+Tab shows a macOS Command-Tab style row of big app icons, with the selected app's name under it.
- **Extras:** overview thumbnails are centered, and there are a Mac-style system monitor and a caffeine applet.

## Install on a new computer

Requirements: Fedora (tested on Fedora 44 with COSMIC 1.8), an internet connection, and a user with `sudo`. COSMIC is installed automatically if missing.

```bash
git clone https://github.com/aqibjavaidabbasi/fedora-cosmic-mac.git
cd fedora-cosmic-mac
./install.sh
```

Then log out, choose **COSMIC** on the login screen, and log back in.

What `install.sh` does:

1. Installs COSMIC (if needed), the build tools, and the Inter font with `dnf`.
2. Downloads the MacTahoe icons and WhiteSur icons and cursors into `~/.local/share/icons`.
3. Installs the extra top-bar applets and GPU Screen Recorder (for the Control Center's Record tiles) with Flatpak.
4. Backs up your current COSMIC settings to `~/.config/cosmic.backup-<date>`.
5. Copies the toolkit to `~/.local/share/cosmic-mac` and the launchers to `~/.local/bin`.
6. Builds the patched COSMIC parts and helper apps from source. The first build takes about 25 to 40 minutes.
7. Applies the Mac settings and turns on automatic rebuilds.
8. Prints a status check.

Your wallpaper and dock apps are kept. To copy the original owner's exact desktop instead, including wallpaper, pinned apps and every setting, run `./install.sh --restore`.

Run `./check.sh` any time to see that everything is active.

## Updating Fedora or COSMIC safely

Always update with the script instead of a plain `dnf upgrade`:

```bash
cd fedora-cosmic-mac
./update.sh
```

What `update.sh` does:

1. Saves the current setup to git and pushes it, so there is a restore point.
2. Runs `sudo dnf upgrade --refresh` and `flatpak update`.
3. Rebuilds every customization for the new COSMIC version. Your settings are not touched.
4. Saves the refreshed setup to git again.
5. Shows the status. Log out and back in afterwards.

If you update some other way (GNOME Software, a plain `dnf upgrade`), nothing breaks. The customizations rebuild on their own at the next login or within a day. Until then, the parts that need rebuilding run as stock COSMIC.

### Upgrading to a new Fedora release

```bash
./update.sh                                     # get fully up to date and backed up first
sudo dnf system-upgrade download --releasever=45
sudo dnf offline reboot
```

After the reboot, run `./update.sh` again. It sees the new Fedora release and rebuilds everything, including the Control Center and Battery apps.

## Why features are not lost on update

- **Session never breaks:** COSMIC starts the launchers in `~/.local/bin` instead of Fedora's programs. A launcher only uses the customized build when it matches the installed COSMIC version, and otherwise falls back to Fedora's own. `apply-settings.sh` keeps `~/.local/bin` first in the login PATH.
- **Automatic rebuild:** when COSMIC is updated, `rebuild.sh` downloads that exact version's source from GitHub, applies the customizations and rebuilds. It is started by the launcher at login and by a daily timer.
- **Two ways to apply each change:**
  - **Normal case:** the scripts in `patches/*.py` make small, tolerant edits that usually still fit a new version.
  - **Fallback:** if a script no longer fits, the exact diff saved in `upstream-patches/<feature>.patch` is merged onto the new version with a git 3-way merge. After each successful build, the diff is saved again for the new version, so the fallback stays current.
- **Library changes:** after a Fedora release upgrade, or if any built program lost a library, everything is rebuilt.
- **Failure handling:** if something still cannot be applied, you get a desktop notification naming the feature. That part keeps running stock COSMIC, and everything else stays customized. `./check.sh` shows which part it is, and `~/.local/share/cosmic-mac/rebuild.log` shows why.

### If a feature fails after an update

Open `~/.local/share/cosmic-mac/rebuild.log` to see which change stopped fitting. Its source is in `~/.local/share/cosmic-mac/build/<component>`, which is the upstream code at the new version. Apply the saved diff there by hand and fix the conflicts:

```bash
cd ~/.local/share/cosmic-mac/build/cosmic-comp
git apply --3way ~/.local/share/cosmic-mac/upstream-patches/compositor.patch
```

Update the matching `patches/*.py` so it fits again, then run `~/.local/share/cosmic-mac/rebuild.sh --force`, `./check.sh`, and `./sync.sh`, and commit.

## Changing the setup

- **Control Center → Edit Controls:** pick the left-most menu bar icon (or any icon name / image path), choose which system monitor values show in the bar, and open the widget settings.
- **Your own tweaks:** change anything in COSMIC Settings as usual. Updates and rebuilds never touch your settings, so your changes are kept.
- **Resetting the Mac look:** run `~/.local/share/cosmic-mac/apply-settings.sh` to apply the Mac defaults again (dock, top bar applets, glass theme, icons, shortcuts). You can edit the file first to change those defaults.
- **Swipe feel:** edit `~/.config/cosmic-mac/animation.conf`, then log out and back in.

Save any change to GitHub:

```bash
./sync.sh && git add -A && git commit -m "Describe the change" && git push
```

## Repository layout

| Path | What it is |
| --- | --- |
| `install.sh` | Sets everything up on a new computer |
| `update.sh` | Backs up, updates Fedora and COSMIC, rebuilds, backs up again |
| `check.sh` | Shows which customizations are active |
| `sync.sh` | Copies the live setup from `$HOME` into this repo |
| `MANIFEST.txt` | COSMIC package versions, Flatpak applets and theme sources |
| `snapshot/home/` | Mirrors `$HOME`. See below |

Inside `snapshot/home/`:

| Path | What it is |
| --- | --- |
| `.local/share/cosmic-mac/` | The toolkit: `rebuild.sh`, `apply-settings.sh`, `patches/`, `upstream-patches/`, and the sources of the Control Center, Battery, maximize helper and theme tool |
| `.local/bin/cosmic-*` | Launchers that pick the customized or the stock COSMIC program |
| `.config/cosmic/` | Every COSMIC setting of the original desktop, used by `--restore` |
| `.config/cosmic-mac/` | Swipe and animation tuning |
| `.config/systemd/user/` | The daily rebuild timer and the wallpaper watcher that recolors the top bar |

Compiled programs, upstream source checkouts, and the large icon themes are not stored. They are rebuilt or downloaded.

## Uninstall

```bash
systemctl --user disable --now cosmic-mac-rebuild.timer cosmic-mac-panel-contrast.path
rm ~/.local/bin/cosmic-comp ~/.local/bin/cosmic-panel ~/.local/bin/cosmic-panel-button \
   ~/.local/bin/cosmic-workspaces ~/.local/bin/cosmic-applet-notifications \
   ~/.local/bin/cosmic-mac-* ~/.local/bin/cosmic-maximize-to-workspace \
   ~/.config/autostart/cosmic-maximize-to-workspace.desktop
```

To bring back the old settings, restore a `~/.config/cosmic.backup-<date>` folder over `~/.config/cosmic`, then log out and back in.
