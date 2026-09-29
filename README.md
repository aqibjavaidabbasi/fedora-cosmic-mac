# cosmic-mac

My macOS-style COSMIC desktop on Fedora. Everything custom lives in this repo, so a COSMIC update can be merged without losing anything.

## What's here

| Path | What it is |
| --- | --- |
| `snapshot/home/` | Mirrors `$HOME`: all COSMIC settings, state, launchers, systemd units, autostart, `.bash_profile` |
| `snapshot/home/.local/share/cosmic-mac/` | The customization toolkit: `patches/*.py`, `rebuild.sh`, `apply-settings.sh`, helper app sources (control center, battery, maximize helper, theme tool) |
| `upstream-patches/*.patch` | Exact `git diff` of every patched COSMIC component against upstream (`BASE-VERSIONS.txt` names the upstream tag) |
| `MANIFEST.txt` | Installed COSMIC rpm versions, flatpak applets, third-party icon/cursor themes and where to get them |
| `sync.sh` | Copies the live setup from `$HOME` into this repo and regenerates the patches |

Not stored: compiled binaries, upstream source checkouts, and the large MacTahoe/WhiteSur themes (see `MANIFEST.txt`). They are rebuilt or reinstalled.

## Saving changes

```bash
./sync.sh && git add -A && git commit -m "Describe the change" && git push
```

## When a COSMIC update arrives

`rebuild.sh` (daily timer) fetches the new upstream tag and re-applies `patches/*.py`. Usually nothing else is needed.

If it reports that a patch no longer applies, merge by hand with the saved diff, for example for the compositor:

```bash
cd ~/.local/share/cosmic-mac/build/cosmic-comp
git apply --3way /path/to/this/repo/upstream-patches/cosmic-comp.patch
```

Resolve any conflicts, update the matching `patches/*.py`, run `rebuild.sh --force`, then `./sync.sh` and commit.

## Restoring on a fresh install

Copy `snapshot/home/` over `$HOME`, install the themes and flatpaks from `MANIFEST.txt`, run `~/.local/share/cosmic-mac/rebuild.sh --force`, enable the `cosmic-mac-*` user units, then log out and back in.
