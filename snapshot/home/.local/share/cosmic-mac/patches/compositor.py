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
     "pub enum SwipeAction {\n\\1NextWorkspace,\n\\1PrevWorkspace,\n\\1Overview,\n\\1HideOverview,\n}"),
])
patch("src/input/mod.rs", [
    # first "Other actions" arm = horizontal workspace layout
    (r"\n([ \t]*)_ => None, // TODO: Other actions\n",
     "\n\\1Some(Direction::Up) => Some(SwipeAction::Overview),\n\\1Some(Direction::Down) => Some(SwipeAction::HideOverview),\n\\1_ => None, // TODO: Other actions\n"),
])
patch("src/input/actions.rs", [
    (r"(SwipeAction::PrevWorkspace => \{\n\s*let _ = to_previous_workspace\([^;]*;\n(\s*)\})",
     "\\1\n\\2SwipeAction::Overview => {\n\\2    if let Some(command) = self.common.config.system_actions.get(&shortcuts::action::System::WorkspaceOverview) {\n\\2        self.spawn_command(command.clone());\n\\2    }\n\\2}\n\\2SwipeAction::HideOverview => {\n\\2    // three fingers down closes the overview (a no-op when it is not open)\n\\2    self.spawn_command(\"busctl --user call com.system76.CosmicWorkspaces /com/system76/CosmicWorkspaces com.system76.CosmicWorkspaces Hide\".to_string());\n\\2}"),
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

# ---------------------------------------------------------------------------
# macOS vibrancy for chosen apps: windows whose app id is listed in
# ~/.config/cosmic-mac/blur-apps (default: org.gnome.Nautilus) get the wallpaper
# and windows behind them blurred, as if the app had asked for it with the
# background-effect protocol (GTK apps can't). Their see-through parts - the
# Files sidebar, styled by gtk/mac-sidebar-*.css - then look frosted.
import re, sys
p = "src/wayland/handlers/background_effect.rs"
s = open(p).read()
s += '''
/// cosmic-mac: apps whose windows get their background blurred like macOS vibrancy.
/// One app id per line in ~/.config/cosmic-mac/blur-apps (read at login).
static MAC_BLUR_APPS: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| {
    let path = format!("{}/.config/cosmic-mac/blur-apps", std::env::var("HOME").unwrap_or_default());
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_string)
            .collect(),
        Err(_) => vec!["org.gnome.Nautilus".to_string()],
    }
});

/// The window-geometry rectangle to blur behind, if this toplevel's app is listed.
pub fn mac_forced_blur(states: &smithay::wayland::compositor::SurfaceData) -> Option<Rectangle<i32, Logical>> {
    let data = states.data_map.get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()?;
    let app_id = data.lock().ok()?.app_id.clone()?;
    if !MAC_BLUR_APPS.iter().any(|a| *a == app_id) {
        return None;
    }
    let geometry = states
        .cached_state
        .get::<smithay::wayland::shell::xdg::SurfaceCachedState>()
        .current()
        .geometry;
    geometry.filter(|g| !g.is_empty())
}
'''
open(p, "w").write(s)

p = "src/backend/render/wayland/blur_effect.rs"
s = open(p).read()
s, n1 = re.subn(
    r"        let mut blur_region_state = states\.cached_state\.get::<ComputedBlurRegionCachedState>\(\);\n        let Some\(region\) = blur_region_state\.current\(\)\.blur_region\.as_ref\(\) else \{\n            return Ok\(None\);\n        \};\n",
    """        // cosmic-mac: listed apps are blurred behind their window geometry
        let forced = crate::wayland::handlers::background_effect::mac_forced_blur(states).map(|r| vec![r]);
        let mut blur_region_state = states.cached_state.get::<ComputedBlurRegionCachedState>();
        let Some(region) = blur_region_state.current().blur_region.as_ref().or(forced.as_ref()) else {
            return Ok(None);
        };
""", s, count=1)
open(p, "w").write(s)

p = "src/shell/element/surface.rs"
s = open(p).read()
s, n2 = re.subn(
    r"(\n    let mut blur_state = data\.cached_state\.get::<ComputedBlurRegionCachedState>\(\);\n    blur_state\n)",
    r"""
    // cosmic-mac: listed apps are blurred behind (see background_effect.rs)
    if crate::wayland::handlers::background_effect::mac_forced_blur(data).is_some() {
        return true;
    }
\1""", s, count=1)
open(p, "w").write(s)
if (n1, n2) != (1, 1):
    print("compositor blur patch failed", n1, n2, file=sys.stderr)
    sys.exit(1)

# ---------------------------------------------------------------------------
# macOS title bars for windows COSMIC decorates itself (server-side decorations:
# Electron/Chromium, Qt and X11 apps): traffic lights on the left - 14px, a little
# larger than macOS so they are easy to hit, grey when the window is in the
# background - and the title centered. Drag to move, double-click to zoom,
# right-click for the window menu.
p = "src/shell/element/window.rs"
s = open(p).read()
s, n3 = re.subn(
    r"(impl Decorations<CosmicWindowInternal, Message> for DefaultDecorations \{\n    fn view\(&self, win: &CosmicWindowInternal\) -> cosmic::Element<'_, Message> \{\n).*?\n        header\.into\(\)\n    \}\n\}",
    r'''\1        use cosmic::iced::{Alignment, Background, Border, Color, Length};
        let sharp_corners = win.window.is_maximized(false)
            || (win.is_tiled() && !win.appearance_conf.lock().unwrap().clip_tiled_windows);
        let focused = win.window.is_activated(false);
        let light = |hex: u32, msg: Option<Message>| {
            let base = Color::from_rgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
            let style = move |dim: f32| cosmic::widget::button::Style {
                background: Some(Background::Color(if focused {
                    Color { r: base.r * dim, g: base.g * dim, b: base.b * dim, a: 1.0 }
                } else {
                    Color::from_rgba(0.5, 0.5, 0.5, 0.35)
                })),
                border_radius: 7.0.into(),
                border_width: 0.5,
                border_color: Color::from_rgba(0.0, 0.0, 0.0, 0.18),
                ..cosmic::widget::button::Style::new()
            };
            cosmic::widget::button::custom(cosmic::widget::Space::new().width(Length::Fixed(14.0)).height(Length::Fixed(14.0)))
                .padding(0)
                .width(Length::Fixed(14.0))
                .height(Length::Fixed(14.0))
                .class(cosmic::theme::Button::Custom {
                    active: Box::new(move |_, _| style(1.0)),
                    hovered: Box::new(move |_, _| style(0.88)),
                    pressed: Box::new(move |_, _| style(0.72)),
                    disabled: Box::new(move |_| style(1.0)),
                })
                .on_press_maybe(msg)
        };
        let lights = cosmic::widget::row::with_children(vec![
            light(0xFF5F57, Some(Message::Close)).into(),
            light(0xFEBC2E, cosmic::config::show_minimize().then_some(Message::Minimize)).into(),
            light(0x28C840, cosmic::config::show_maximize().then_some(Message::Maximize)).into(),
        ])
        .spacing(8)
        .align_y(Alignment::Center);
        let title = cosmic::widget::text::heading(win.last_title.lock().unwrap().clone())
            .size(13)
            .wrapping(cosmic::iced::widget::text::Wrapping::None);
        let bar = cosmic::iced::widget::Stack::new()
            .push(
                cosmic::widget::container(title)
                    .center_x(Length::Fill)
                    .center_y(Length::Fill)
                    .padding([0, 90]),
            )
            .push(cosmic::widget::container(lights).padding([0, 12]).center_y(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill);
        let radius = if sharp_corners { 0.0 } else { 10.0 };
        cosmic::widget::mouse_area(
            cosmic::widget::container(bar)
                .width(Length::Fill)
                .height(Length::Fill)
                .class(cosmic::theme::Container::custom(move |t| {
                    let c = t.cosmic();
                    let mut bg: Color = c.background(false).base.into();
                    if !focused {
                        bg = Color { r: bg.r * 0.97 + 0.015, g: bg.g * 0.97 + 0.015, b: bg.b * 0.97 + 0.015, a: bg.a };
                    }
                    let on: Color = c.on_bg_color().into();
                    cosmic::iced::widget::container::Style {
                        background: Some(Background::Color(bg)),
                        text_color: Some(if focused { on } else { Color { a: 0.5, ..on } }),
                        border: Border { radius: [radius, radius, 0.0, 0.0].into(), ..Default::default() },
                        ..Default::default()
                    }
                })),
        )
        .on_press(Message::DragStart)
        .on_double_click(Message::Maximize)
        .on_right_press(Message::Menu)
        .into()
    }
}''', s, count=1, flags=re.S)
if n3 != 1:
    print("compositor title bar patch failed", file=sys.stderr)
    sys.exit(1)
open(p, "w").write(s)

# ---------------------------------------------------------------------------
# Smoother blur: the scratch texture used while blurring is kept between frames instead of
# being allocated (at the size of the blurred area, up to a whole 4K screen) and freed on
# every frame for every blurred surface, and the passes no longer wait on the GPU in between
# (everything runs in one GL context, so the commands are already ordered).
p = "src/backend/render/wayland/blur_effect.rs"
s = open(p).read()
s, b1 = re.subn(
    r"        let mut off_texture = renderer\n            \.as_mut\(\)\n            \.create_buffer\(Fourcc::Abgr8888, tex_size\)\n            \.map_err\(R::from_gles_error\)\?;\n",
    """        let scratch_ref = cache.get_or_insert_threadsafe(BlurScratch::default);
        let mut scratch = scratch_ref.lock().unwrap();
        if scratch.as_ref().is_some_and(|t| t.size() != tex_size) || texture_is_new {
            *scratch = None;
        }
        let mut off_texture = match scratch.clone() {
            Some(t) => t,
            None => {
                let t: GlesTexture = renderer
                    .as_mut()
                    .create_buffer(Fourcc::Abgr8888, tex_size)
                    .map_err(R::from_gles_error)?;
                *scratch = Some(t.clone());
                t
            }
        };
""", s, count=1)
s, b2 = re.subn(
    r"        if texture_entry\.is_none\(\) \{\n            let gl_texture = renderer",
    "        let texture_is_new = texture_entry.is_none();\n        if texture_entry.is_none() {\n            let gl_texture = renderer",
    s, count=1)
s, b3 = re.subn(
    r"(\nfn blit_from_active_fb\()",
    """
/// The second texture the blur passes ping-pong with, kept per blurred surface.
type BlurScratch = std::sync::Mutex<Option<GlesTexture>>;
\\1""", s, count=1)
s, b4 = re.subn(
    r"        let sync = frame\.finish\(\)\?;\n        std::mem::drop\(fb\);\n        renderer\.wait\(&sync\)\?;\n",
    "        let _sync = frame.finish()?;\n        std::mem::drop(fb);\n", s)
if (b1, b2, b3) != (1, 1, 1) or b4 != 2:
    print("blur speed-up patch failed", (b1, b2, b3, b4), file=sys.stderr)
    sys.exit(1)
open(p, "w").write(s)

# ---------------------------------------------------------------------------
# Stage A1 (behavior-neutral): pure recognizer and workspace-transition modules, and
# `Spring::velocity_at`. Nothing in the running compositor uses them yet (steps A2 to A4 will),
# so the compositor behaves exactly as before. Unit tests live in the new files
# (`cargo test --lib`).
import os, shutil
here = os.path.dirname(os.path.abspath(__file__))
shutil.copy(os.path.join(here, "mac_recognizer.rs"), "src/input/gestures/recognizer.rs")
shutil.copy(os.path.join(here, "mac_workspace_transition.rs"), "src/shell/workspace_transition.rs")

patch("src/input/gestures/mod.rs", [
    (r"(use cosmic_settings_config::shortcuts::action::Direction;\n)",
     "pub mod recognizer;\n\n\\1"),
])
patch("src/shell/mod.rs", [
    (r"(pub mod grabs;\n)", "\\1pub mod workspace_transition;\n"),
])
patch("src/backend/render/animations/spring.rs", [
    (r"(    pub fn value_at\(&self, t: Duration\) -> f64 \{\n        self\.oscillate\(t\.as_secs_f64\(\)\)\n    \}\n)",
     """\\1
    /// Velocity of the spring (the derivative of `value_at`) at time `t`, in the same units per
    /// second as the position. At `t = 0` this is `initial_velocity`.
    pub fn velocity_at(&self, t: Duration) -> f64 {
        self.derivative(t.as_secs_f64())
    }

    fn derivative(&self, t: f64) -> f64 {
        let b = self.params.damping;
        let m = self.params.mass;
        let k = self.params.stiffness;
        let v0 = self.initial_velocity;

        let beta = b / (2. * m);
        let omega0 = (k / m).sqrt();
        let x0 = self.from - self.to;
        let envelope = (-beta * t).exp();
        // c = beta * x0 + v0 appears in every regime
        let c = beta * x0 + v0;

        if (beta - omega0).abs() <= f64::from(f32::EPSILON) {
            // Critically damped.
            envelope * (v0 - beta * c * t)
        } else if beta < omega0 {
            // Underdamped.
            let omega1 = ((omega0 * omega0) - (beta * beta)).sqrt();
            let a = c / omega1;
            envelope * (v0 * (omega1 * t).cos() + (-x0 * omega1 - beta * a) * (omega1 * t).sin())
        } else {
            // Overdamped.
            let omega2 = ((beta * beta) - (omega0 * omega0)).sqrt();
            let a = c / omega2;
            envelope * (v0 * (omega2 * t).cosh() + (x0 * omega2 - beta * a) * (omega2 * t).sinh())
        }
    }
"""),
])

# ---------------------------------------------------------------------------
# Stage A2: the shell uses the retargetable `Transition` for shortcut / protocol / activation
# switching (a new request starts from the current visual position and velocity instead of
# replacing the running animation and jumping). The live three/four-finger SWIPE path is left
# exactly as it was: it keeps using `previously_active` and its own pair model. Rendering and
# pointer hit-testing take the pair to draw from `Workspaces::render_pair`.
def replace_exact(path, old, new, count=1):
    s = open(path).read()
    if s.count(old) != count:
        print(f"A2 patch failed in {path}: {old[:70]!r} found {s.count(old)}x", file=sys.stderr)
        sys.exit(1)
    open(path, "w").write(s.replace(old, new))

SH = "src/shell/mod.rs"
replace_exact(SH,
    "pub struct WorkspaceSet {\n    previously_active: Option<(usize, WorkspaceDelta)>,\n",
    "pub struct WorkspaceSet {\n    /// The live swipe's animation pair (unchanged by Stage A2).\n    previously_active: Option<(usize, WorkspaceDelta)>,\n"
    "    /// Retargetable transition used for every other kind of workspace switch.\n    transition: workspace_transition::Transition,\n")
replace_exact(SH,
    "        WorkspaceSet {\n            previously_active: None,\n            active: 0,\n",
    "        WorkspaceSet {\n            previously_active: None,\n            transition: workspace_transition::Transition::new(0),\n            active: 0,\n")
replace_exact(SH,
    """            self.previously_active = if animate {
                Some((old_active, workspace_delta))
            } else {
                None
            };
            self.active = idx;
""",
    """            let now = Instant::now();
            if self.transition.is_idle() {
                // at rest the transition sits on the workspace we are leaving (indices may have
                // changed since it last moved)
                self.transition.snap(old_active);
            }
            self.previously_active = None;
            match workspace_delta {
                // everything except the live swipe: retarget from the current visual state
                WorkspaceDelta::Shortcut(_) if animate => {
                    self.transition.retarget_timed(now, idx, ANIMATION_DURATION)
                }
                // the live swipe keeps its own pair model for now
                WorkspaceDelta::Gesture { .. } | WorkspaceDelta::GestureEnd { .. } => {
                    self.transition.snap(idx);
                    if animate {
                        self.previously_active = Some((old_active, workspace_delta));
                    }
                }
                // no animation (the overview is open)
                WorkspaceDelta::Shortcut(_) => self.transition.snap(idx),
            }
            self.active = idx;
""")
replace_exact(SH,
    """        } else {
            self.workspaces[self.active].refresh();
        }
        self.sticky_layer.refresh();
""",
    """        } else {
            self.transition.refresh(Instant::now());
            // like before, the active workspace is only refreshed while nothing is animating
            if self.transition.is_idle() {
                self.workspaces[self.active].refresh();
            }
        }
        self.sticky_layer.refresh();
""")
replace_exact(SH,
    """            set.previously_active
                .as_ref()
                .is_some_and(|(_, delta)| delta.is_animating())
                || set.sticky_layer.animations_going()
""",
    """            set.previously_active
                .as_ref()
                .is_some_and(|(_, delta)| delta.is_animating())
                || set.transition.is_animating(Instant::now())
                || set.sticky_layer.animations_going()
""")
# the pair to draw, per set and per Workspaces
replace_exact(SH,
    "    fn update_workspace_delta(&mut self, delta: f64, forward: bool) {\n",
    """    /// What to draw and hit-test now: the workspace shown as "current" and, while switching,
    /// the one leaving (`previous`), as `(handle, index, delta)`. The live swipe's pair wins when
    /// there is one; otherwise it comes from the transition.
    fn render_pair(
        &self,
        now: Instant,
    ) -> (
        Option<(WorkspaceHandle, usize, WorkspaceDelta)>,
        (WorkspaceHandle, usize),
    ) {
        let current = (self.workspaces[self.active].handle, self.active);
        if let Some((idx, delta)) = self.previously_active {
            return (
                self.workspaces.get(idx).map(|previous| (previous.handle, idx, delta)),
                current,
            );
        }
        if self.transition.is_idle() {
            return (None, current);
        }
        let view = self.transition.view(now, self.workspaces.len());
        let pair = workspace_transition::pair_view(
            &view,
            self.active,
            self.workspaces.len().saturating_sub(1),
            self.transition.direction(now),
        );
        let Some(current) = self.workspaces.get(pair.current).map(|w| (w.handle, pair.current))
        else {
            return (None, (self.workspaces[self.active].handle, self.active));
        };
        let previous = pair.previous.and_then(|previous| {
            self.workspaces.get(previous.idx).map(|w| {
                (
                    w.handle,
                    previous.idx,
                    WorkspaceDelta::Gesture {
                        percentage: previous.percentage as f64,
                        forward: previous.forward,
                    },
                )
            })
        });
        (previous, current)
    }

    fn update_workspace_delta(&mut self, delta: f64, forward: bool) {
""")
replace_exact(SH,
    "    pub fn active_mut(&mut self, output: &Output) -> Option<&mut Workspace> {\n",
    """    /// The pair to render and hit-test for `output`: `(previous, current)` as handles and
    /// indices. Use this instead of combining `active()` and `active_num()`, whose first
    /// element only knows the live swipe's pair.
    pub fn render_pair(
        &self,
        output: &Output,
    ) -> Option<(
        Option<(WorkspaceHandle, usize, WorkspaceDelta)>,
        (WorkspaceHandle, usize),
    )> {
        self.sets
            .get(output)
            .or(self.backup_set.as_ref())
            .map(|set| set.render_pair(Instant::now()))
    }

    pub fn active_mut(&mut self, output: &Output) -> Option<&mut Workspace> {
""")

RENDER = "src/backend/render/mod.rs"
replace_exact(RENDER,
    """    let Some((previous_workspace, workspace)) = shell_guard.workspaces.active(output) else {
        #[cfg(not(feature = "debug"))]
        return Ok(Vec::new());
        #[cfg(feature = "debug")]
        return Ok(debug_elements);
    };

    let (previous_idx, idx) = shell_guard.workspaces.active_num(output);
    let previous_workspace = previous_workspace
        .zip(previous_idx)
        .map(|((w, start), idx)| (w.handle, idx, start));
    let workspace = (workspace.handle, idx);
""",
    """    let Some((previous_workspace, workspace)) = shell_guard.workspaces.render_pair(output) else {
        #[cfg(not(feature = "debug"))]
        return Ok(Vec::new());
        #[cfg(feature = "debug")]
        return Ok(debug_elements);
    };
""")
replace_exact(RENDER,
    """    let (previous_workspace, workspace) = shell_ref
        .workspaces
        .active(output)
        .ok_or(RenderError::OutputNoMode(OutputNoMode))?;
    let (previous_idx, idx) = shell_ref.workspaces.active_num(output);
    let previous_workspace = previous_workspace
        .zip(previous_idx)
        .map(|((w, start), idx)| (w.handle, idx, start));
    let workspace = (workspace.handle, idx);
""",
    """    let (previous_workspace, workspace) = shell_ref
        .workspaces
        .render_pair(output)
        .ok_or(RenderError::OutputNoMode(OutputNoMode))?;
""")
INPUT = "src/input/mod.rs"
replace_exact(INPUT,
    """        let (previous_workspace, workspace) = shell.workspaces.active(output)?;
        let (previous_idx, idx) = shell.workspaces.active_num(output);
        let previous_workspace = previous_workspace
            .zip(previous_idx)
            .map(|((w, start), idx)| (w.handle, idx, start));
        let workspace = (workspace.handle, idx);
""",
    """        let (previous_workspace, workspace) = shell.workspaces.render_pair(output)?;
""", count=2)

# ---------------------------------------------------------------------------
# Stage A3: compositor-owned swipe gestures.
#   - ownership is decided once at GestureSwipeBegin (3 or 4 fingers and no pointer grab),
#     never because the overview happens to be open
#   - the A1 recognizer (dead zone, axis ratio, axis lock, cancellation) drives a `Session`
#   - horizontal swipes drive the retargetable workspace transition (rubber band at the ends,
#     decision by position and velocity in workspace units, cancel never activates)
#   - vertical swipes open/close the overview once, through the compositor's own D-Bus session
#     connection (no shell, no new process)
#   - the old swipe machinery (`previously_active`, first-event direction, `end_workspace_swipe`)
#     is removed
shutil.copy(os.path.join(here, "mac_gesture_session.rs"), "src/input/gestures/session.rs")

def replace_between(path, start_marker, end_marker, new, include_end=False):
    s = open(path).read()
    a = s.find(start_marker)
    if a < 0 or s.count(start_marker) != 1:
        print(f"A3 patch failed in {path}: start {start_marker[:60]!r} x{s.count(start_marker)}", file=sys.stderr)
        sys.exit(1)
    b = s.find(end_marker, a + len(start_marker))
    if b < 0:
        print(f"A3 patch failed in {path}: end {end_marker[:60]!r}", file=sys.stderr)
        sys.exit(1)
    if include_end:
        b += len(end_marker)
    open(path, "w").write(s[:a] + new + s[b:])

# ---- gestures/mod.rs: keep RubberBand, drop the old first-event GestureState --------------------
G = "src/input/gestures/mod.rs"
g = open(G).read()
i = g.index("// rubber_band.rs from Niri")
open(G, "w").write('''pub mod recognizer;
pub mod session;

use smithay::output::Output;
use std::sync::LazyLock;

/// Who owns the swipe in progress. A compositor-owned swipe remembers the output it began on.
pub type GestureOwner = session::Owner<Output>;

/// Gesture tuning, read once from the environment (`COSMIC_MAC_GESTURE_*`, `COSMIC_MAC_SWIPE_LENGTH`).
/// The defaults are provisional until they are set from real touchpad recordings.
pub fn gesture_tuning() -> &'static session::Tuning {
    static TUNING: LazyLock<session::Tuning> = LazyLock::new(|| {
        session::Tuning::from_lookup(|name| std::env::var(name).ok().and_then(|v| v.trim().parse().ok()))
    });
    &TUNING
}

''' + g[i:])

# ---- state.rs -------------------------------------------------------------------------------
ST = "src/state.rs"
replace_exact(ST, "    input::{PointerFocusState, gestures::GestureState},\n", "    input::{PointerFocusState, gestures::GestureOwner},\n")
replace_exact(ST, "    pub gesture_state: Option<GestureState>,\n", "    pub gesture_owner: GestureOwner,\n")
replace_exact(ST, "                gesture_state: None,\n", "                gesture_owner: GestureOwner::None,\n")

# ---- dbus: a fire-and-forget call on the compositor's own session connection ---------------------
replace_exact("src/dbus/mod.rs",
    "    fn spawn(&self, fut: impl Future<Output = ()> + 'static) {\n",
    r'''    /// Call a method without arguments on the session bus and ignore the reply. Uses the
    /// compositor's existing session connection; nothing is spawned and nothing blocks.
    pub fn call_session_method(
        &self,
        destination: &'static str,
        path: &'static str,
        interface: &'static str,
        method: &'static str,
    ) {
        let state = self.clone();
        self.spawn(async move {
            match state.session_conn().await {
                Ok(conn) => {
                    if let Err(err) = conn
                        .call_method(Some(destination), path, Some(interface), method, &())
                        .await
                    {
                        let text = err.to_string();
                        let hint = if text.contains("ServiceUnknown") || text.contains("NameHasNoOwner") {
                            "the service is not running or has not claimed its name"
                        } else {
                            "the service answered with an error"
                        };
                        tracing::warn!(%err, destination, path, interface, method, "Session D-Bus call failed: {hint}");
                    }
                }
                Err(err) => tracing::warn!(?err, "No session D-Bus connection"),
            }
        });
    }

    fn spawn(&self, fut: impl Future<Output = ()> + 'static) {
''')

# ---- input/mod.rs: the swipe events ---------------------------------------------------------------
IN = "src/input/mod.rs"
replace_exact(IN, "        gestures::{GestureState, SwipeAction},\n",
    "        gestures::{\n            gesture_tuning,\n            recognizer::Input as GestureInput,\n            session::OwnerKind,\n        },\n")
SWIPE = r'''            InputEvent::GestureSwipeBegin { event, .. } => {
                let maybe_seat = self
                    .common
                    .shell
                    .read()
                    .seats
                    .for_device(&event.device(), &backend_id)
                    .cloned();
                if let Some(seat) = maybe_seat {
                    self.common.idle_notifier_state.notify_activity(&seat);
                    let pointer = seat.get_pointer().unwrap();
                    // Ownership is decided here, once: 3 or 4 fingers and no pointer grab means
                    // the compositor owns the gesture, anything else goes to the client. The
                    // overview being open is deliberately not part of the decision.
                    let output = seat.active_output();
                    let previous = self.common.gesture_owner.begin(
                        event.fingers(),
                        pointer.is_grabbed(),
                        gesture_tuning(),
                        output,
                    );
                    if let Some((previous_output, action)) = previous {
                        // a compositor session was still alive: abort it safely (never commits)
                        self.apply_gesture_action(&previous_output, action);
                    }
                    if self.common.gesture_owner.kind() == OwnerKind::Client {
                        let serial = SERIAL_COUNTER.next_serial();
                        pointer.gesture_swipe_begin(
                            self,
                            &GestureSwipeBeginEvent {
                                serial,
                                time: event.time(),
                                fingers: event.fingers(),
                            },
                        );
                    }
                }
            }
            InputEvent::GestureSwipeUpdate { event, .. } => {
                let maybe_seat = self
                    .common
                    .shell
                    .read()
                    .seats
                    .for_device(&event.device(), &backend_id)
                    .cloned();
                if let Some(seat) = maybe_seat {
                    self.common.idle_notifier_state.notify_activity(&seat);
                    if self.common.gesture_owner.kind() == OwnerKind::Compositor {
                        self.feed_compositor_gesture(GestureInput::Update {
                            dx: event.delta().x,
                            dy: event.delta().y,
                            time: Duration::from_millis(event.time().millis() as u64),
                        });
                    } else {
                        let pointer = seat.get_pointer().unwrap();
                        pointer.gesture_swipe_update(
                            self,
                            &GestureSwipeUpdateEvent {
                                time: event.time(),
                                delta: event.delta(),
                            },
                        );
                    }
                }
            }
            InputEvent::GestureSwipeEnd { event, .. } => {
                let maybe_seat = self
                    .common
                    .shell
                    .read()
                    .seats
                    .for_device(&event.device(), &backend_id)
                    .cloned();
                if let Some(seat) = maybe_seat {
                    self.common.idle_notifier_state.notify_activity(&seat);
                    if self.common.gesture_owner.kind() == OwnerKind::Compositor {
                        // a cancelled gesture is cancelled, never completed
                        self.feed_compositor_gesture(GestureInput::End {
                            cancelled: event.cancelled(),
                            time: Duration::from_millis(event.time().millis() as u64),
                        });
                    } else {
                        let serial = SERIAL_COUNTER.next_serial();
                        let pointer = seat.get_pointer().unwrap();
                        pointer.gesture_swipe_end(
                            self,
                            &GestureSwipeEndEvent {
                                serial,
                                time: event.time(),
                                cancelled: event.cancelled(),
                            },
                        );
                    }
                    self.common.gesture_owner.end();
                }
            }
'''
replace_between(IN,
    "            InputEvent::GestureSwipeBegin { event, .. } => {\n",
    "            InputEvent::GesturePinchBegin { event, .. } => {\n",
    SWIPE)

# ---- input/actions.rs: helpers replace handle_swipe_action; shortcuts lose the gesture flag ------
AC = "src/input/actions.rs"
replace_exact(AC, "use super::gestures;\n",
    "use super::gestures::{\n    GestureOwner,\n    recognizer::Input as GestureInput,\n    session::{Action as GestureAction, Context as GestureContext, overview_call},\n};\nuse crate::utils::quirks::workspace_overview_is_open;\nuse smithay::output::Output;\n")
HELPERS = r'''    /// What a compositor-owned swipe needs to know about the compositor right now.
    pub fn gesture_context(&self, output: &Output) -> GestureContext {
        let natural_scroll = self
            .common
            .config
            .cosmic_conf
            .input_touchpad
            .scroll_config
            .as_ref()
            .and_then(|scroll| scroll.natural_scroll)
            .unwrap_or(false);
        GestureContext {
            natural_scroll,
            vertical_layout: self.common.config.cosmic_conf.workspaces.workspace_layout
                == WorkspaceLayout::Vertical,
            overview_open: workspace_overview_is_open(output),
        }
    }

    /// Feed one event to the compositor-owned swipe and apply what it asks for.
    pub fn feed_compositor_gesture(&mut self, input: GestureInput) {
        let GestureOwner::Compositor { mut session, extra: output } =
            std::mem::take(&mut self.common.gesture_owner)
        else {
            return;
        };
        let context = self.gesture_context(&output);
        let action = session.feed(input, &context);
        if !matches!(input, GestureInput::End { .. }) {
            self.common.gesture_owner = GestureOwner::Compositor { session, extra: output.clone() };
        }
        self.apply_gesture_action(&output, action);
    }

    /// Apply one action of a compositor-owned swipe.
    pub fn apply_gesture_action(&mut self, output: &Output, action: GestureAction) {
        match action {
            GestureAction::None => {}
            GestureAction::WorkspaceTrack { first, total, velocity } => {
                self.common.shell.write().workspace_swipe_track(output, first, total, velocity);
            }
            GestureAction::WorkspaceFinish { velocity } => {
                self.common.shell.write().workspace_swipe_finish(
                    output,
                    velocity,
                    &mut self.common.workspace_state.update(),
                );
            }
            GestureAction::WorkspaceCancel => {
                self.common.shell.write().workspace_swipe_cancel(output);
            }
            GestureAction::OverviewShow | GestureAction::OverviewHide => {
                // The D-Bus call is fire-and-forget on the compositor's own session connection.
                // If `cosmic-workspaces` is not there or answers with an error, DBusState logs a
                // warning; nothing else changes (no workspace or gesture state is involved).
                if let Some(call) = overview_call(action) {
                    self.common.dbus_state.call_session_method(
                        call.destination,
                        call.path,
                        call.interface,
                        call.method,
                    );
                }
            }
        }
        if matches!(
            action,
            GestureAction::WorkspaceTrack { .. }
                | GestureAction::WorkspaceFinish { .. }
                | GestureAction::WorkspaceCancel
        ) {
            let outputs = self.common.shell.read().workspace_swipe_outputs(output);
            for output in outputs {
                self.backend.schedule_render(&output);
            }
        }
    }

'''
replace_between(AC,
    "    pub fn handle_swipe_action(&mut self, action: gestures::SwipeAction, seat: &Seat<State>) {\n",
    "    #[profiling::function]\n    pub fn handle_shortcut_action(",
    HELPERS)
replace_exact(AC, "    seat: &Seat<State>,\n    gesture: bool,\n    wraparound: bool,\n", "    seat: &Seat<State>,\n    wraparound: bool,\n", count=2)
replace_exact(AC, """        if gesture {
            WorkspaceDelta::new_gesture(true)
        } else {
            WorkspaceDelta::new_shortcut()
        },
""", "        WorkspaceDelta::new_shortcut(),\n")
replace_exact(AC, """        if gesture {
            WorkspaceDelta::new_gesture(false)
        } else {
            WorkspaceDelta::new_shortcut()
        },
""", "        WorkspaceDelta::new_shortcut(),\n")
replace_exact(AC, "                let next = to_next_workspace(\n                    &mut self.common.shell.write(),\n                    seat,\n                    false,\n",
                  "                let next = to_next_workspace(\n                    &mut self.common.shell.write(),\n                    seat,\n")
replace_exact(AC, "                let previous = to_previous_workspace(\n                    &mut self.common.shell.write(),\n                    seat,\n                    false,\n",
                  "                let previous = to_previous_workspace(\n                    &mut self.common.shell.write(),\n                    seat,\n")

# ---- shell/mod.rs ---------------------------------------------------------------------------------
replace_exact(SH, "static GESTURE_MAX_LENGTH_CFG: std::sync::LazyLock<f64> =\n    std::sync::LazyLock::new(|| mac_env(\"COSMIC_MAC_SWIPE_LENGTH\", 150.0).max(10.0));\n", "")
replace_exact(SH, "const GESTURE_POSITION_THRESHOLD: f64 = 0.5;\nconst GESTURE_VELOCITY_THRESHOLD: f64 = 0.02;\n", "")
replace_exact(SH, "    backend::render::animations::spring::{Spring, SpringParams},\n", "    backend::render::animations::spring::SpringParams,\n    input::gestures::{RubberBand, gesture_tuning},\n")
replace_between(SH,
    "#[derive(Debug, Clone, Copy)]\npub enum WorkspaceDelta {\n",
    "#[derive(Debug)]\npub struct WorkspaceSet {\n",
    r'''#[derive(Debug, Clone, Copy)]
pub enum WorkspaceDelta {
    /// A switch requested by a shortcut, the workspace protocol or an activation.
    Shortcut(Instant),
    /// How the renderer is told about the workspace that is leaving: `percentage` of the way out,
    /// moving `forward` (toward a lower x) or not. Produced from the transition each frame.
    Gesture { percentage: f64, forward: bool },
}

impl WorkspaceDelta {
    pub fn new_shortcut() -> Self {
        WorkspaceDelta::Shortcut(Instant::now())
    }
}

''')
replace_exact(SH,
    "pub struct WorkspaceSet {\n    /// The live swipe's animation pair (unchanged by Stage A2).\n    previously_active: Option<(usize, WorkspaceDelta)>,\n    /// Retargetable transition used for every other kind of workspace switch.\n    transition: workspace_transition::Transition,\n",
    "pub struct WorkspaceSet {\n    /// Every workspace switch (shortcut, protocol, swipe) is a position in workspace-index space.\n    transition: workspace_transition::Transition,\n")
replace_exact(SH, "            previously_active: None,\n            transition: workspace_transition::Transition::new(0),\n", "            transition: workspace_transition::Transition::new(0),\n")
replace_between(SH,
    "    fn activate(\n        &mut self,\n        idx: usize,\n        workspace_delta: WorkspaceDelta,\n",
    "    /// What to draw and hit-test now: the workspace shown as \"current\"",
    r'''    /// Make `idx` the logical active workspace (protocol state and `active`) without touching
    /// the visual transition. Returns the workspace that was active.
    fn set_active(
        &mut self,
        idx: usize,
        state: &mut WorkspaceUpdateGuard<'_, State>,
    ) -> usize {
        let old_active = self.active;
        state.remove_workspace_state(&self.workspaces[old_active].handle, WState::Active);
        state.remove_workspace_state(&self.workspaces[old_active].handle, WState::Urgent);
        state.remove_workspace_state(&self.workspaces[idx].handle, WState::Urgent);
        state.add_workspace_state(&self.workspaces[idx].handle, WState::Active);
        self.active = idx;
        old_active
    }

    /// Switch to `idx` for a shortcut, a protocol request or an activation. The animation
    /// starts from the current visual position and velocity (a running one is retargeted, never
    /// replaced). `workspace_delta` is only kept so those callers need no change.
    fn activate(
        &mut self,
        idx: usize,
        _workspace_delta: WorkspaceDelta,
        state: &mut WorkspaceUpdateGuard<'_, State>,
    ) -> Result<bool, InvalidWorkspaceIndex> {
        if idx >= self.workspaces.len() {
            return Err(InvalidWorkspaceIndex);
        }

        // Animate if workspaces overview isn't open
        let animate = {
            let layer_map = layer_map_for_output(&self.output);
            !layer_map
                .layers()
                .any(|l| l.namespace() == WORKSPACE_OVERVIEW_NAMESPACE)
        };

        if self.active != idx {
            let now = Instant::now();
            let old_active = self.set_active(idx, state);
            if self.transition.is_idle() {
                // at rest the transition sits on the workspace we are leaving (indices may have
                // changed since it last moved)
                self.transition.snap(old_active);
            }
            if animate {
                self.transition.retarget_timed(now, idx, ANIMATION_DURATION);
            } else {
                self.transition.snap(idx);
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// A swipe is tracking workspaces. `first` begins tracking from the current visual state
    /// (position and velocity of a running animation are adopted, never reset to `active`).
    fn swipe_track(&mut self, now: Instant, first: bool, total: f64, velocity: f64) {
        let tuning = gesture_tuning();
        if first {
            self.transition.begin_tracking(now, self.active);
        }
        let last = self.workspaces.len().saturating_sub(1);
        let band = RubberBand {
            stiffness: tuning.band_stiffness,
            limit: tuning.band_limit,
        };
        self.transition.track(total, velocity, last, &band);
    }

    /// The fingers lifted: pick the target and animate there. Only the target becomes the
    /// logical active workspace, and only now. Ignored when something else (a shortcut, say)
    /// took the transition over during the swipe.
    fn swipe_finish(
        &mut self,
        now: Instant,
        velocity: f64,
        state: &mut WorkspaceUpdateGuard<'_, State>,
    ) {
        if !self.transition.is_tracking() {
            return;
        }
        let tuning = gesture_tuning();
        let last = self.workspaces.len().saturating_sub(1);
        let target = self.transition.finish(
            now,
            self.active,
            velocity,
            last,
            workspace_transition::DecideParams {
                velocity_threshold: tuning.velocity_threshold,
                position_threshold: tuning.position_threshold,
            },
            swipe_spring(),
        );
        if target != self.active {
            self.set_active(target, state);
        }
    }

    /// The swipe was cancelled: animate back to the logical active workspace, activate nothing.
    fn swipe_cancel(&mut self, now: Instant) {
        if self.transition.is_tracking() {
            self.transition.cancel(now, self.active, swipe_spring());
        }
    }

''')
replace_exact(SH,
    "    /// What to draw and hit-test now: the workspace shown as \"current\" and, while switching,\n    /// the one leaving (`previous`), as `(handle, index, delta)`. The live swipe's pair wins when\n    /// there is one; otherwise it comes from the transition.\n",
    "    /// What to draw and hit-test now: the workspace shown as \"current\" and, while switching,\n    /// the one leaving (`previous`), as `(handle, index, delta)`, derived from the transition.\n")
replace_exact(SH,
    "        if let Some((idx, delta)) = self.previously_active {\n            return (\n                self.workspaces.get(idx).map(|previous| (previous.handle, idx, delta)),\n                current,\n            );\n        }\n", "")
replace_between(SH,
    "    fn update_workspace_delta(&mut self, delta: f64, forward: bool) {\n",
    "    fn set_output(&mut self, new_output: &Output, explicit: bool) {\n", "")
replace_between(SH,
    "    fn refresh(&mut self) {\n        if let Some((_, start)) = self.previously_active {\n",
    "        self.sticky_layer.refresh();\n    }\n",
    '''    fn refresh(&mut self) {
        self.transition.refresh(Instant::now());
        // like before, the active workspace is only refreshed while nothing is animating
        if self.transition.is_idle() {
            self.workspaces[self.active].refresh();
        }
        self.sticky_layer.refresh();
    }
''', include_end=True)
replace_exact(SH,
    """                (
                    set.previously_active.and_then(|(idx, start)| {
                        set.workspaces.get(idx).map(|previous| (previous, start))
                    }),
                    &set.workspaces[set.active],
                )
""",
    """                (
                    // the workspace that is leaving, as the renderer pairs it with the current one
                    set.render_pair(Instant::now()).0.and_then(|(_, idx, delta)| {
                        set.workspaces.get(idx).map(|previous| (previous, delta))
                    }),
                    &set.workspaces[set.active],
                )
""")
replace_exact(SH,
    "        (set.previously_active.map(|(idx, _)| idx), set.active)\n",
    "        (set.render_pair(Instant::now()).0.map(|(_, idx, _)| idx), set.active)\n")
replace_exact(SH,
    """            set.previously_active
                .as_ref()
                .is_some_and(|(_, delta)| delta.is_animating())
                || set.transition.is_animating(Instant::now())
                || set.sticky_layer.animations_going()
""",
    """            set.transition.is_animating(Instant::now()) || set.sticky_layer.animations_going()
""")
replace_between(SH,
    "    pub fn update_workspace_delta(&mut self, output: &Output, delta: f64, forward: bool) {\n",
    "    /// Handle a client's ext-workspace create request",
    "")
replace_between(SH,
    "    pub fn end_workspace_swipe(\n",
    "    pub fn active_space(&self, output: &Output) -> Option<&Workspace> {\n",
    r'''    /// The outputs whose workspace sets move together with `output` during a swipe.
    pub fn workspace_swipe_outputs(&self, output: &Output) -> Vec<Output> {
        match self.workspaces.mode {
            WorkspaceMode::OutputBound => vec![output.clone()],
            WorkspaceMode::Global => self.workspaces.sets.keys().cloned().collect(),
        }
    }

    fn workspace_swipe_sets(&mut self, output: &Output) -> Vec<&mut WorkspaceSet> {
        match self.workspaces.mode {
            WorkspaceMode::OutputBound => self.workspaces.sets.get_mut(output).into_iter().collect(),
            WorkspaceMode::Global => self.workspaces.sets.values_mut().collect(),
        }
    }

    /// A workspace swipe is tracking. `total` and `velocity` are in workspace units (positive =
    /// toward a higher index); `first` begins tracking from the current visual state.
    pub fn workspace_swipe_track(&mut self, output: &Output, first: bool, total: f64, velocity: f64) {
        let now = Instant::now();
        for set in self.workspace_swipe_sets(output) {
            set.swipe_track(now, first, total, velocity);
        }
    }

    /// The fingers lifted after a workspace swipe: settle on the chosen workspace.
    pub fn workspace_swipe_finish(
        &mut self,
        output: &Output,
        velocity: f64,
        workspace_state: &mut WorkspaceUpdateGuard<'_, State>,
    ) {
        let now = Instant::now();
        let cleanup = matches!(self.workspaces.mode, WorkspaceMode::OutputBound)
            && matches!(
                self.overview_mode.active_trigger(),
                Some(Trigger::Pointer(_) | Trigger::Touch(_) | Trigger::Tool(_, _))
            );
        for set in self.workspace_swipe_sets(output) {
            if cleanup {
                set.workspaces[set.active].tiling_layer.cleanup_drag();
            }
            set.swipe_finish(now, velocity, workspace_state);
        }
    }

    /// A workspace swipe was cancelled: go back to the logical workspace, activate nothing.
    pub fn workspace_swipe_cancel(&mut self, output: &Output) {
        let now = Instant::now();
        for set in self.workspace_swipe_sets(output) {
            set.swipe_cancel(now);
        }
    }

''')
# the release spring (existing tuning knobs), used by the swipe methods
replace_exact(SH, "static MAC_SPACES: std::sync::LazyLock<bool> =",
    "fn swipe_spring() -> SpringParams {\n    SpringParams::new(*SPRING_DAMPING_CFG, *SPRING_STIFFNESS_CFG, 0.0001)\n}\nstatic MAC_SPACES: std::sync::LazyLock<bool> =")

# unused imports left behind in input/mod.rs, and the renderer's match on the removed variant
replace_exact(IN, "use cosmic_comp_config::{NumlockState, workspace::WorkspaceLayout};\n", "use cosmic_comp_config::NumlockState;\n")
replace_exact(IN, "use cosmic_settings_config::shortcuts::action::{Direction, ResizeDirection};\n", "use cosmic_settings_config::shortcuts::action::ResizeDirection;\n")
replace_exact("src/shell/focus/order.rs", """                WorkspaceDelta::GestureEnd {
                    start,
                    spring,
                    forward,
                } => (
                    *forward,
                    (spring.value_at(Instant::now().duration_since(*start)) as f32).clamp(0.0, 1.0),
                ),
""", "")

# ---------------------------------------------------------------------------
# Tray popovers: windows of apps listed in ~/.config/cosmic-mac/tray-popovers
# (default: Clippy) open like macOS menu-bar popovers. They float, sit just
# below the top bar centered on the pointer (which is on the tray icon that was
# just clicked), and always take keyboard focus. Stock COSMIC ignores where X11
# apps ask to be placed, and Clippy hides itself the moment it lacks focus.
replace_exact(SH, "#[derive(Debug)]\npub struct PendingWindow {", '''/// cosmic-mac: app ids (one per line, case-insensitive) whose windows open like
/// macOS menu-bar popovers, from ~/.config/cosmic-mac/tray-popovers (read at login).
static MAC_TRAY_POPOVERS: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| {
    let path = format!("{}/.config/cosmic-mac/tray-popovers", std::env::var("HOME").unwrap_or_default());
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_lowercase)
            .collect(),
        Err(_) => vec!["clippy".to_string()],
    }
});

fn mac_is_tray_popover(window: &CosmicSurface) -> bool {
    let app_id = window.app_id().to_lowercase();
    !app_id.is_empty() && MAC_TRAY_POPOVERS.iter().any(|a| *a == app_id)
}

/// Output-local spot for a tray popover: centered on the pointer, 6px below the top bar.
fn mac_tray_popover_position(
    seat: &Seat<State>,
    output: &Output,
    size: Size<i32, Logical>,
) -> Option<Point<i32, Local>> {
    let pointer = seat.get_pointer()?.current_location();
    let output_loc = output.geometry().loc;
    let zone = layer_map_for_output(output).non_exclusive_zone();
    let x = (pointer.x as i32 - output_loc.x - size.w / 2)
        .min(zone.loc.x + zone.size.w - size.w - 6)
        .max(zone.loc.x + 6);
    Some(Point::from((x, zone.loc.y + 6)))
}

#[derive(Debug)]
pub struct PendingWindow {''')
replace_exact(SH,
    "        let floating_exception = layout::has_floating_exception(&self.tiling_exceptions, &window);\n",
    "        let floating_exception = layout::has_floating_exception(&self.tiling_exceptions, &window);\n"
    "        let mac_popover = mac_is_tray_popover(&window);\n")
replace_exact(SH,
    "            && !(workspace.is_tiled(&focused.active_window()) && floating_exception)\n",
    "            && !(workspace.is_tiled(&focused.active_window()) && floating_exception)\n"
    "            && !mac_popover\n")
replace_exact(SH,
    "        if is_dialog || floating_exception || !workspace.tiling_enabled {\n"
    "            workspace.floating_layer.map(mapped.clone(), None);\n",
    "        if is_dialog || floating_exception || mac_popover || !workspace.tiling_enabled {\n"
    "            let position = mac_popover\n"
    "                .then(|| mac_tray_popover_position(&seat, &workspace_output, mapped.geometry().size))\n"
    "                .flatten();\n"
    "            workspace.floating_layer.map(mapped.clone(), position);\n")
replace_exact(SH,
    "        } else if (workspace_output == seat.active_output() && active_handle == workspace_handle)\n"
    "            || should_be_sticky\n",
    "        } else if (workspace_output == seat.active_output() && active_handle == workspace_handle)\n"
    "            || should_be_sticky\n"
    "            || mac_popover\n")
