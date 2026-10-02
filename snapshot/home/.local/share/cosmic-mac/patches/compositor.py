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
