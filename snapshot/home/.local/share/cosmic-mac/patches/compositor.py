#!/usr/bin/env python3
"""Patches for cosmic-comp (run from its source root). Exits 1 if the code changed.

1. Three-finger swipes switch workspaces (like four-finger ones).
2. Mac-like workspace swipe feel, tunable through environment variables
   (set from ~/.config/cosmic-mac/animation.conf by the cosmic-comp launcher):
     COSMIC_MAC_SWIPE_LENGTH      finger travel for a full switch  (stock 150)
     COSMIC_MAC_SPRING_STIFFNESS  finish spring stiffness          (stock 1000)
     COSMIC_MAC_SPRING_DAMPING    finish spring damping ratio      (stock 1.0)
"""
import re, sys

def patch(path, pairs):
    s = open(path).read()
    for pat, rep in pairs:
        s, n = re.subn(pat, rep, s, count=1)
        if n != 1:
            print(f"patch failed in {path}: {pat[:60]}", file=sys.stderr)
            sys.exit(1)
    open(path, "w").write(s)

patch("src/input/mod.rs", [
    (r"\n([ \t]*)3 => None, // TODO: 3 finger gestures\n[ \t]*4 => \{", r"\n\g<1>3 | 4 => {"),
])

patch("src/shell/mod.rs", [
    (r"const GESTURE_MAX_LENGTH: f64 = [0-9.]+;",
     """fn mac_env(name: &str, default: f64) -> f64 {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}
static GESTURE_MAX_LENGTH_CFG: std::sync::LazyLock<f64> =
    std::sync::LazyLock::new(|| mac_env("COSMIC_MAC_SWIPE_LENGTH", 150.0).max(10.0));
static SPRING_STIFFNESS_CFG: std::sync::LazyLock<f64> =
    std::sync::LazyLock::new(|| mac_env("COSMIC_MAC_SPRING_STIFFNESS", 1000.0).max(10.0));
static SPRING_DAMPING_CFG: std::sync::LazyLock<f64> =
    std::sync::LazyLock::new(|| mac_env("COSMIC_MAC_SPRING_DAMPING", 1.0).max(0.1));"""),
    (r"let easing = delta\.clamp\(0\.0, GESTURE_MAX_LENGTH\)\.abs\(\) / GESTURE_MAX_LENGTH;",
     "let easing = delta.clamp(0.0, *GESTURE_MAX_LENGTH_CFG).abs() / *GESTURE_MAX_LENGTH_CFG;"),
    (r"SpringParams::new\(1\.0, 1000\.0, 0\.0001\)",
     "SpringParams::new(*SPRING_DAMPING_CFG, *SPRING_STIFFNESS_CFG, 0.0001)"),
])

# 3. Three/four-finger swipe up opens the workspaces overview (Mission Control).
patch("src/input/gestures/mod.rs", [
    (r"pub enum SwipeAction \{\n(\s*)NextWorkspace,\n\s*PrevWorkspace,\n\}",
     "pub enum SwipeAction {\n\\1NextWorkspace,\n\\1PrevWorkspace,\n\\1Overview,\n}"),
])
patch("src/input/mod.rs", [
    # first "Other actions" arm = horizontal workspace layout
    (r"\n([ \t]*)_ => None, // TODO: Other actions\n",
     "\n\\1Some(Direction::Up) => Some(SwipeAction::Overview),\n\\1_ => None, // TODO: Other actions\n"),
])
patch("src/input/actions.rs", [
    (r"(SwipeAction::PrevWorkspace => \{\n\s*let _ = to_previous_workspace\([^;]*;\n(\s*)\})",
     "\\1\n\\2SwipeAction::Overview => {\n\\2    if let Some(command) = self.common.config.system_actions.get(&shortcuts::action::System::WorkspaceOverview) {\n\\2        self.spawn_command(command.clone());\n\\2    }\n\\2}"),
])

# 4. "Mac Spaces" (COSMIC_MAC_SPACES=1): no spare empty workspace at the end.
#    Each display keeps its first (desktop) workspace plus workspaces that have
#    windows; empty ones are removed at once (new ones get a 3 s grace period).
#    Workspace-create requests from clients are honored.
patch("src/shell/mod.rs", [
    (r"(static SPRING_DAMPING_CFG[^;]*;)",
     """\\1
static MAC_SPACES: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| std::env::var("COSMIC_MAC_SPACES").is_ok_and(|v| v.trim() == "1"));
static MAC_RECENT: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<WorkspaceHandle, Instant>>> =
    std::sync::LazyLock::new(Default::default);"""),
    (r"(fn ensure_last_empty\(\n\s*&mut self,\n\s*state: &mut WorkspaceUpdateGuard<State>,\n\s*xdg_activation_state: &XdgActivationState,\n\s*\) \{\n)",
     """\\1        if *MAC_SPACES {
            self.mac_prune(state, xdg_activation_state);
            return;
        }
"""),
    (r"(\n    fn update_workspace_idxs\(&self)",
     """
    fn mac_prune(
        &mut self,
        state: &mut WorkspaceUpdateGuard<State>,
        xdg_activation_state: &XdgActivationState,
    ) {
        if self.workspaces.is_empty() {
            self.add_empty_workspace(state);
            return;
        }
        let now = Instant::now();
        let recent = MAC_RECENT.lock().unwrap().clone();
        let kept: Vec<bool> = self
            .workspaces
            .iter()
            .enumerate()
            .map(|(i, workspace)| {
                let fresh = recent
                    .get(&workspace.handle)
                    .is_some_and(|t| now.duration_since(*t) < Duration::from_secs(3));
                let keep = i == 0
                    || i == self.active
                    || fresh
                    || !workspace.can_auto_remove(xdg_activation_state);
                if !keep {
                    state.remove_workspace(workspace.handle);
                }
                keep
            })
            .collect();
        let mut iter = kept.iter();
        self.workspaces.retain(|_| *iter.next().unwrap());
        self.active -= kept
            .iter()
            .take(self.active + 1)
            .filter(|kept| !**kept)
            .count();
        if kept.iter().any(|val| !(*val)) {
            self.update_workspace_idxs(state);
        }
    }
\\1"""),
    (r"(\n    pub fn end_workspace_swipe\()",
     """
    /// Handle a client's ext-workspace create request: add an empty workspace
    /// at the end of that display's list.
    pub fn mac_create_workspace(
        &mut self,
        group: &WorkspaceGroupHandle,
        state: &mut WorkspaceUpdateGuard<'_, State>,
    ) {
        if let Some(set) = self.workspaces.sets.values_mut().find(|s| &s.group == group) {
            set.add_empty_workspace(state);
            if let Some(w) = set.workspaces.last() {
                MAC_RECENT.lock().unwrap().insert(w.handle, Instant::now());
            }
        }
    }
\\1"""),
])
patch("src/wayland/handlers/workspace.rs", [
    (r"(\n(\s*)Request::SetPin \{)",
     """\n\\2Request::Create { in_group, .. } => {
\\2    let mut shell = self.common.shell.write();
\\2    shell.mac_create_workspace(&in_group, &mut self.common.workspace_state.update());
\\2}\\1"""),
])

# 5. Moving a maximized window to another workspace (toplevel-management
#    move_to_workspace, used by cosmic-maximize-to-workspace) keeps it maximized
#    without a second maximize animation, like COSMIC's own move_element does.
patch("src/shell/mod.rs", [
    (r"""                let position = match window_state \{
                    WorkspaceRestoreData::Floating\(data\) => Some\(
                        data\.position_relative\(to_workspace\.output\.geometry\(\)\.size\.as_logical\(\)\),
                    \),
                    _ => None,
                \};
                to_workspace\.floating_layer\.map\(mapped\.clone\(\), position\);
                mapped\.into\(\)""",
     """                let (position, was_maximized) = match &window_state {
                    WorkspaceRestoreData::Floating(data) => (
                        Some(data.position_relative(to_workspace.output.geometry().size.as_logical())),
                        data.was_maximized,
                    ),
                    _ => (None, false),
                };
                to_workspace.floating_layer.map(mapped.clone(), position);
                if was_maximized {
                    if let Some(geometry) = to_workspace.floating_layer.element_geometry(&mapped) {
                        *mapped.maximized_state.lock().unwrap() = Some(MaximizedState {
                            original_geometry: geometry,
                            original_layer: ManagedLayer::Floating,
                            original_snapped: None,
                        });
                        to_workspace.floating_layer.map_maximized(mapped.clone(), geometry, false);
                    }
                }
                mapped.into()"""),
])
