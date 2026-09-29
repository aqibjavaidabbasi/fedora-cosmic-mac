#!/bin/bash
# Rebuilds the three-finger-swipe COSMIC compositor and the maximize helper
# for whatever cosmic-comp version Fedora has installed, then re-applies settings.
# Run by the daily timer and by the cosmic-comp launcher when versions differ.
# Use --force to rebuild even if the version already matches.

D=$HOME/.local/share/cosmic-mac
LOG=$D/rebuild.log
exec 9>"$D/.lock"
flock -n 9 || { echo "A rebuild is already running."; exit 0; }
exec >"$LOG" 2>&1
set -uo pipefail

notify() { notify-send -a "COSMIC Mac setup" -i system-software-update "$1" "${2:-}" 2>/dev/null || true; }
fail() { echo "FAILED: $1"; notify "COSMIC rebuild failed" "$1 — details in $LOG"; exit 1; }

VER=$(rpm -q --qf '%{VERSION}' cosmic-comp) || fail "cosmic-comp is not installed"
HAVE=$(cat "$D/built-version" 2>/dev/null || true)
AVER_NOW=$(rpm -q --qf '%{VERSION}' cosmic-applets 2>/dev/null || true)
AHAVE=$(cat "$D/applets-built-version" 2>/dev/null || true)
PVER_OK=$([ "$(rpm -q --qf '%{VERSION}' cosmic-panel 2>/dev/null)" = "$(cat "$D/cosmic-panel-built-version" 2>/dev/null)" ] && echo 1)
WVER_OK=$([ "$(rpm -q --qf '%{VERSION}' cosmic-workspaces 2>/dev/null)" = "$(cat "$D/cosmic-workspaces-built-version" 2>/dev/null)" ] && echo 1)
if [ "$VER" = "$HAVE" ] && [ "$AVER_NOW" = "$AHAVE" ] && [ -n "$PVER_OK" ] && [ -n "$WVER_OK" ] && [ "${1:-}" != "--force" ]; then
    echo "Already built for COSMIC $VER (applets $AVER_NOW)."
    exit 0
fi
echo "$(date): rebuilding for COSMIC $VER (had: ${HAVE:-none}), applets $AVER_NOW (had: ${AHAVE:-none})"
notify "Updating your COSMIC customizations" "Rebuilding for COSMIC $VER. This can take about 25 minutes; COSMIC works normally meanwhile."

export RUSTC_BOOTSTRAP=1 CARGO_NET_GIT_FETCH_WITH_CLI=true
export CARGO_TARGET_DIR=$HOME/.cache/cosmic-mac/target

if [ "$VER" != "$HAVE" ] || [ "${1:-}" = "--force" ]; then

# --- Compositor with the three-finger patch --------------------------------
SRC=$D/build/cosmic-comp
rm -rf "$SRC" && mkdir -p "$SRC" && cd "$SRC" || fail "cannot create build folder"
git init -q && git remote add origin https://github.com/pop-os/cosmic-comp.git
git fetch -q --depth 1 origin "refs/tags/epoch-$VER:refs/tags/epoch-$VER" \
    || fail "COSMIC $VER source (tag epoch-$VER) not found on GitHub"
git -c advice.detachedHead=false checkout -q "epoch-$VER^{commit}" || fail "checkout failed"

python3 "$D/patches/compositor.py" || fail "the COSMIC patches no longer apply to this version"
grep -q "3 | 4 => {" src/input/mod.rs || fail "patch check failed"

nice -n 19 cargo build --release --locked -Zgit=shallow-deps \
    || fail "build failed (if it mentions a missing library, run: sudo dnf builddep cosmic-comp)"
install -Dm755 "$CARGO_TARGET_DIR/release/cosmic-comp" "$D/bin/cosmic-comp.new" && mv -f "$D/bin/cosmic-comp.new" "$D/bin/cosmic-comp"

# --- Maximize helper, built against the same protocol revision -------------
REV=$(grep -A2 'name = "cosmic-protocols"' Cargo.lock | grep -o '#[0-9a-f]\{40\}' | tr -d '#' | head -1)
cd "$D/maximize-helper"
[ -n "$REV" ] && sed -i "s/rev = \"[0-9a-f]\{40\}\"/rev = \"$REV\"/" Cargo.toml
if nice -n 19 cargo build --release -Zgit=shallow-deps; then
    install -m755 "$CARGO_TARGET_DIR/release/cosmic-maximize-to-workspace" "$HOME/.local/bin/cosmic-maximize-to-workspace"
else
    notify "Maximize-to-workspace helper failed to rebuild" "The old one is kept. Details in $LOG"
fi

# --- Theme tool, built against the same libcosmic revision --------------------
LIBCOSMIC=$(grep -A2 'name = "cosmic-theme"' "$SRC/Cargo.lock" | grep -o '#[0-9a-f]\{40\}' | tr -d '#' | head -1)
cd "$D/theme-tool"
[ -n "$LIBCOSMIC" ] && sed -i "s/rev = \"[0-9a-f]\{40\}\"/rev = \"$LIBCOSMIC\"/g" Cargo.toml
if nice -n 19 cargo build --release -Zgit=shallow-deps; then
    install -Dm755 "$CARGO_TARGET_DIR/release/cosmic-mac-theme" "$D/bin/cosmic-mac-theme"
else
    notify "Glass theme tool failed to rebuild" "The old one is kept. Details in $LOG"
fi

echo "$VER" > "$D/built-version"
fi
# --- Panel buttons: per-button icon/text (cosmic-applets) --------------------
AVER=$(rpm -q --qf '%{VERSION}' cosmic-applets 2>/dev/null)
if [ -n "$AVER" ] && { [ "$AVER" != "$(cat "$D/applets-built-version" 2>/dev/null)" ] || [ "${1:-}" = "--force" ]; }; then
    ASRC=$D/build/cosmic-applets
    if rm -rf "$ASRC" && mkdir -p "$ASRC" && cd "$ASRC" && git init -q \
        && git remote add origin https://github.com/pop-os/cosmic-applets.git \
        && git fetch -q --depth 1 origin "refs/tags/epoch-$AVER:refs/tags/epoch-$AVER" \
        && git -c advice.detachedHead=false checkout -q "epoch-$AVER^{commit}" \
        && python3 "$D/patches/panel-button.py" \
        && CARGO_TARGET_DIR=$HOME/.cache/cosmic-mac/target-applets nice -n 19 cargo build --release --locked -p cosmic-panel-button -Zgit=shallow-deps; then
        install -Dm755 "$HOME/.cache/cosmic-mac/target-applets/release/cosmic-panel-button" "$D/bin/cosmic-panel-button"
        echo "$AVER" > "$D/applets-built-version"
        # Notification Center (clock + calendar + notifications) from the same source
        if python3 "$D/patches/notifications.py" \
            && CARGO_TARGET_DIR=$HOME/.cache/cosmic-mac/target-applets nice -n 19 cargo build --release -p cosmic-applet-notifications -Zgit=shallow-deps; then
            install -Dm755 "$HOME/.cache/cosmic-mac/target-applets/release/cosmic-applet-notifications" "$D/bin/cosmic-applet-notifications"
            echo "$AVER" > "$D/cosmic-applet-notifications-built-version"
        else
            notify "Notification Center patch failed to rebuild" "COSMIC's standard notifications applet is used meanwhile. Details in $LOG"
        fi
    else
        notify "Top bar button patch failed to rebuild" "COSMIC's standard buttons are used meanwhile. Details in $LOG"
    fi
fi

# --- Top bar (recolor without restart) and overview (centered thumbnails) -----
# rebuild_extra <rpm> <github repo> <patch> <cargo -p package or ""> <binary>
rebuild_extra() {
    local rpm=$1 repo=$2 patch=$3 pkg=$4 bin=$5
    local v; v=$(rpm -q --qf '%{VERSION}' "$rpm" 2>/dev/null) || return 0
    [ "$v" = "$(cat "$D/$bin-built-version" 2>/dev/null)" ] && [ "${FORCE:-}" != 1 ] && return 0
    local src=$D/build/$rpm tdir=$HOME/.cache/cosmic-mac/target-$bin
    if rm -rf "$src" && mkdir -p "$src" && cd "$src" && git init -q \
        && git remote add origin "https://github.com/pop-os/$repo.git" \
        && git fetch -q --depth 1 origin "refs/tags/epoch-$v:refs/tags/epoch-$v" \
        && git -c advice.detachedHead=false checkout -q "epoch-$v^{commit}" \
        && python3 "$D/patches/$patch" \
        && CARGO_TARGET_DIR=$tdir nice -n 19 cargo build --release --locked ${pkg:+-p $pkg} -Zgit=shallow-deps; then
        install -Dm755 "$tdir/release/$bin" "$D/bin/$bin"
        echo "$v" > "$D/$bin-built-version"
    else
        notify "$bin patch failed to rebuild" "COSMIC's standard $bin is used meanwhile. Details in $LOG"
    fi
}
[ "${1:-}" = "--force" ] && FORCE=1
rebuild_extra cosmic-panel cosmic-panel panel.py cosmic-panel-bin cosmic-panel
rebuild_extra cosmic-workspaces cosmic-workspaces-epoch workspaces.py "" cosmic-workspaces

"$D/apply-settings.sh"
echo "$(date): done"
notify "COSMIC customizations updated" "Log out and back into COSMIC to use the three-finger swipe again."
