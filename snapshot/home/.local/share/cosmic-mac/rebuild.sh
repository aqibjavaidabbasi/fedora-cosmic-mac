#!/bin/bash
# Rebuilds the patched COSMIC components (compositor, panel buttons, Notification
# Center, top bar, overview, Spotlight launcher) for whatever COSMIC version Fedora has installed,
# plus the helper apps. It never touches your settings.
# Run by the daily timer and by the launchers when versions differ.
# Use --force to rebuild everything even if versions already match.
#
# Keeping features across updates:
#  - Each feature is applied by patches/<name>.py (tolerant regex edits).
#  - If a script no longer matches a new COSMIC version, the exact diff saved in
#    upstream-patches/<name>.patch is merged onto the new version (git 3-way).
#  - After every successful build the diff is saved again for the new version.
#  - After a Fedora release upgrade, or if a binary lost a library, everything
#    (including the Control Center, Battery and maximize helper) is rebuilt.
#  - Until a rebuild succeeds the launchers run Fedora's stock component, so the
#    session never breaks.

D=$HOME/.local/share/cosmic-mac
U=$D/upstream-patches
LOG=$D/rebuild.log
exec 9>"$D/.lock"
flock -n 9 || { echo "A rebuild is already running."; exit 0; }
exec > >(tee "$LOG") 2>&1
set -uo pipefail

notify() { notify-send -a "COSMIC Mac setup" -i system-software-update "$1" "${2:-}" 2>/dev/null || true; }
fail() { echo "FAILED: $1"; notify "COSMIC rebuild failed" "$1 — details in $LOG"; exit 1; }
FAILED=()

VER=$(rpm -q --qf '%{VERSION}' cosmic-comp) || fail "cosmic-comp is not installed"
AVER=$(rpm -q --qf '%{VERSION}' cosmic-applets 2>/dev/null || true)
REL=$(. /etc/os-release; echo "$VERSION_ID")

FORCE=
[ "${1:-}" = "--force" ] && FORCE=1
# New Fedora release: libraries may have changed, rebuild everything.
[ "$REL" != "$(cat "$D/os-built-version" 2>/dev/null)" ] && { echo "Fedora release changed to $REL"; FORCE=1; }
# Any built binary missing a shared library: rebuild everything.
for b in "$D"/bin/* "$HOME"/.local/bin/cosmic-mac-* "$HOME/.local/bin/cosmic-maximize-to-workspace"; do
    [ -x "$b" ] && file -b "$b" | grep -q ELF && ldd "$b" 2>/dev/null | grep -q "not found" \
        && { echo "$b is missing a library"; FORCE=1; }
done

stale() { [ -n "$FORCE" ] || [ "$1" != "$(cat "$D/$2" 2>/dev/null)" ]; }
if ! stale "$VER" built-version && ! stale "$AVER" applets-built-version \
   && ! stale "$AVER" cosmic-applet-notifications-built-version \
   && ! stale "$AVER" cosmic-app-list-built-version \
   && ! stale "$(rpm -q --qf '%{VERSION}' cosmic-panel 2>/dev/null)" cosmic-panel-built-version \
   && ! stale "$(rpm -q --qf '%{VERSION}' cosmic-workspaces 2>/dev/null)" cosmic-workspaces-built-version \
   && ! stale "$(rpm -q --qf '%{VERSION}' cosmic-launcher 2>/dev/null)" cosmic-launcher-built-version \
   && [ -x "$HOME/.local/bin/cosmic-mac-control-center" ] && [ -x "$HOME/.local/bin/cosmic-mac-battery" ] \
   && [ -x "$HOME/.local/bin/cosmic-mac-wifi" ] && [ -x "$HOME/.local/bin/cosmic-mac-bluetooth" ] \
   && [ -x "$HOME/.local/bin/cosmic-mac-stats" ]; then
    echo "Already built for COSMIC $VER (applets $AVER, Fedora $REL)."
    exit 0
fi
echo "$(date): rebuilding for COSMIC $VER, applets $AVER, Fedora $REL"
notify "Updating your COSMIC customizations" "Rebuilding for COSMIC $VER. This can take about 25 minutes; COSMIC works normally meanwhile."

export RUSTC_BOOTSTRAP=1 CARGO_NET_GIT_FETCH_WITH_CLI=true
export CARGO_TARGET_DIR=$HOME/.cache/cosmic-mac/target
mkdir -p "$D/bin" "$U"

# checkout <folder> <github repo> <tag>: fresh shallow checkout of an upstream tag
checkout() {
    rm -rf "$1" && mkdir -p "$1" && cd "$1" && git init -q \
        && git remote add origin "https://github.com/pop-os/$2.git" \
        && git fetch -q --depth 1 origin "refs/tags/$3:refs/tags/$3" \
        && git -c advice.detachedHead=false checkout -q "$3^{commit}"
}

# apply_feature <name> <paths>: run patches/<name>.py in the current source tree;
# if it no longer matches, 3-way merge the saved diff from the version it was made for.
apply_feature() {
    local name=$1 paths=$2
    python3 "$D/patches/$name.py" && return 0
    echo "patches/$name.py no longer matches; merging the saved $name.patch instead"
    git checkout -q -- $paths 2>/dev/null
    local p=$U/$name.patch base
    base=$(cat "$U/$name.base" 2>/dev/null)
    [ -s "$p" ] && [ -n "$base" ] || return 1
    git fetch -q --depth 1 origin "refs/tags/$base:refs/tags/$base" || return 1
    if git apply --3way "$p"; then
        notify "Merged the $name customization onto the new COSMIC" "The patch script needs updating, but the feature was kept. See $LOG"
        return 0
    fi
    echo "Merge of $name.patch has conflicts:"; git diff --name-only --diff-filter=U
    git reset -q --hard HEAD
    return 1
}

# save_feature <name> <paths>: remember the exact working diff for the next update
save_feature() {
    git diff HEAD -- $2 ':(exclude)Cargo.lock' > "$U/$1.patch.new" && [ -s "$U/$1.patch.new" ] \
        && mv -f "$U/$1.patch.new" "$U/$1.patch" && git describe --tags > "$U/$1.base"
    rm -f "$U/$1.patch.new"
}

# --- Compositor: three-finger swipe, Mac swipe feel, Mac Spaces --------------
if stale "$VER" built-version; then
    SRC=$D/build/cosmic-comp
    if checkout "$SRC" cosmic-comp "epoch-$VER" && apply_feature compositor . \
        && grep -q "3 | 4 => {" src/input/mod.rs \
        && nice -n 19 cargo build --release --locked -Zgit=shallow-deps; then
        install -Dm755 "$CARGO_TARGET_DIR/release/cosmic-comp" "$D/bin/cosmic-comp.new" && mv -f "$D/bin/cosmic-comp.new" "$D/bin/cosmic-comp"
        save_feature compositor .
        echo "$VER" > "$D/built-version"
    else
        FAILED+=("compositor (three-finger swipe)")
    fi

    # Maximize helper and theme tool, built against the same protocol/libcosmic revisions
    if [ -f "$SRC/Cargo.lock" ]; then
        REV=$(grep -A2 'name = "cosmic-protocols"' "$SRC/Cargo.lock" | grep -o '#[0-9a-f]\{40\}' | tr -d '#' | head -1)
        LIBCOSMIC=$(grep -A2 'name = "cosmic-theme"' "$SRC/Cargo.lock" | grep -o '#[0-9a-f]\{40\}' | tr -d '#' | head -1)
        cd "$D/maximize-helper"
        [ -n "$REV" ] && sed -i "s/rev = \"[0-9a-f]\{40\}\"/rev = \"$REV\"/" Cargo.toml
        if nice -n 19 cargo build --release -Zgit=shallow-deps; then
            install -m755 "$CARGO_TARGET_DIR/release/cosmic-maximize-to-workspace" "$HOME/.local/bin/cosmic-maximize-to-workspace.new" \
                && mv -f "$HOME/.local/bin/cosmic-maximize-to-workspace.new" "$HOME/.local/bin/cosmic-maximize-to-workspace"
        else
            FAILED+=("maximize-to-workspace helper")
        fi
        cd "$D/theme-tool"
        [ -n "$LIBCOSMIC" ] && sed -i "s/rev = \"[0-9a-f]\{40\}\"/rev = \"$LIBCOSMIC\"/g" Cargo.toml
        if nice -n 19 cargo build --release -Zgit=shallow-deps; then
            install -Dm755 "$CARGO_TARGET_DIR/release/cosmic-mac-theme" "$D/bin/cosmic-mac-theme"
        else
            FAILED+=("glass theme tool")
        fi
    fi
fi

# --- Panel buttons (per-button icon/text) and Notification Center ------------
if [ -n "$AVER" ] && { stale "$AVER" applets-built-version || stale "$AVER" cosmic-applet-notifications-built-version || stale "$AVER" cosmic-app-list-built-version; }; then
    ASRC=$D/build/cosmic-applets
    AT=$HOME/.cache/cosmic-mac/target-applets
    if checkout "$ASRC" cosmic-applets "epoch-$AVER"; then
        if apply_feature panel-button cosmic-panel-button \
            && CARGO_TARGET_DIR=$AT nice -n 19 cargo build --release --locked -p cosmic-panel-button -Zgit=shallow-deps; then
            install -Dm755 "$AT/release/cosmic-panel-button" "$D/bin/cosmic-panel-button"
            save_feature panel-button cosmic-panel-button
            echo "$AVER" > "$D/applets-built-version"
        else
            FAILED+=("top-left bar buttons")
        fi
        if apply_feature notifications cosmic-applet-notifications \
            && CARGO_TARGET_DIR=$AT nice -n 19 cargo build --release -p cosmic-applet-notifications -Zgit=shallow-deps; then
            install -Dm755 "$AT/release/cosmic-applet-notifications" "$D/bin/cosmic-applet-notifications"
            save_feature notifications cosmic-applet-notifications
            echo "$AVER" > "$D/cosmic-applet-notifications-built-version"
        else
            FAILED+=("Notification Center")
        fi
        if apply_feature dock cosmic-app-list \
            && CARGO_TARGET_DIR=$AT nice -n 19 cargo build --release --locked -p cosmic-app-list -Zgit=shallow-deps; then
            install -Dm755 "$AT/release/cosmic-app-list" "$D/bin/cosmic-app-list"
            save_feature dock cosmic-app-list
            echo "$AVER" > "$D/cosmic-app-list-built-version"
        else
            FAILED+=("Dock icons (spacing + magnification)")
        fi
    else
        FAILED+=("cosmic-applets source download")
    fi
fi

# --- Top bar (recolor without restart) and overview (centered thumbnails) -----
# rebuild_extra <rpm> <github repo> <feature> <cargo -p package or ""> <binary>
rebuild_extra() {
    local rpm=$1 repo=$2 feature=$3 pkg=$4 bin=$5
    local v; v=$(rpm -q --qf '%{VERSION}' "$rpm" 2>/dev/null) || return 0
    stale "$v" "$bin-built-version" || return 0
    local src=$D/build/$rpm tdir=$HOME/.cache/cosmic-mac/target-$bin
    if checkout "$src" "$repo" "epoch-$v" && apply_feature "$feature" . \
        && CARGO_TARGET_DIR=$tdir nice -n 19 cargo build --release --locked ${pkg:+-p $pkg} -Zgit=shallow-deps; then
        install -Dm755 "$tdir/release/$bin" "$D/bin/$bin"
        save_feature "$feature" .
        echo "$v" > "$D/$bin-built-version"
    else
        FAILED+=("$bin")
    fi
}
rebuild_extra cosmic-panel cosmic-panel panel cosmic-panel-bin cosmic-panel
rebuild_extra cosmic-workspaces cosmic-workspaces-epoch workspaces "" cosmic-workspaces
rebuild_extra cosmic-launcher cosmic-launcher launcher "" cosmic-launcher

# --- Control Center and Battery (standalone; rebuilt when missing or forced) ---
build_app() {
    local dir=$1 bin=$2
    [ -z "$FORCE" ] && [ -x "$HOME/.local/bin/$bin" ] && return 0
    cd "$D/$dir" || { FAILED+=("$bin"); return; }
    if CARGO_TARGET_DIR=$HOME/.cache/cosmic-mac/target-helper nice -n 19 cargo build --release -Zgit=shallow-deps; then
        install -m755 "$HOME/.cache/cosmic-mac/target-helper/release/$bin" "$HOME/.local/bin/$bin.new" \
            && mv -f "$HOME/.local/bin/$bin.new" "$HOME/.local/bin/$bin"
    else
        FAILED+=("$bin")
    fi
}
build_app control-center cosmic-mac-control-center
build_app battery cosmic-mac-battery
build_app wifi cosmic-mac-wifi
build_app bluetooth cosmic-mac-bluetooth
build_app stats cosmic-mac-stats
build_app separator cosmic-mac-separator

# Settings are NOT re-applied here, so anything changed in COSMIC Settings survives
# updates. Only the per-display top bars are refreshed from their template.
"$D/panel-contrast.py" >/dev/null 2>&1 || true

if [ ${#FAILED[@]} -eq 0 ]; then
    echo "$REL" > "$D/os-built-version"
    echo "$(date): done"
    notify "COSMIC customizations updated" "Log out and back into COSMIC to use them."
else
    echo "$(date): done, but these failed (stock COSMIC is used for them meanwhile): ${FAILED[*]}"
    notify "Some COSMIC customizations need attention" "Not rebuilt: ${FAILED[*]}. Details in $LOG"
    exit 1
fi
