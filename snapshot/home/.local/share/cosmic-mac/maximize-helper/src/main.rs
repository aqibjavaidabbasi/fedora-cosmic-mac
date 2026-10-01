//! macOS-like Spaces + menu bar behaviour for COSMIC.
//!
//! Workspaces (on every display independently):
//! - The leftmost workspace is the "desktop": it is pinned (never removed) and
//!   holds all normal, un-maximized windows.
//! - Maximizing or full-screening a window moves it to its own new workspace on
//!   the right, and shows it.
//! - Restoring (un-maximizing) it moves it back to the desktop workspace and
//!   shows the desktop. Closing it also returns to the desktop.
//!
//! Top bars (one per display, created from the "Panel" template):
//! - When a display shows a maximized/fullscreen app, its bar takes that app's
//!   color (sampled from the top of the window; darkened or lightened when it would
//!   clash with the theme's text color). Always a custom color: switching between a
//!   custom color and the Dark/Light styles would rebuild the bar and reload every applet.
//! - When a display shows the desktop, its bar is frosted glass tinted with
//!   the wallpaper's color (panel-contrast.py).

use std::{
    collections::{HashMap, HashSet},
    fs,
    time::Instant,
};

use cosmic_client_toolkit::{
    cosmic_protocols::{
        toplevel_info::v1::client::zcosmic_toplevel_handle_v1::State as TState,
        toplevel_management::v1::client::zcosmic_toplevel_manager_v1,
        workspace::v2::client::zcosmic_workspace_handle_v2,
    },
    screencopy::{
        CaptureFrame, CaptureOptions, CaptureSession, CaptureSource, FailureReason, Formats,
        Frame, ScreencopyFrameData, ScreencopyFrameDataExt, ScreencopyHandler,
        ScreencopySessionData, ScreencopySessionDataExt, ScreencopyState,
    },
    sctk::{
        self,
        output::{OutputHandler, OutputState},
        registry::{ProvidesRegistryState, RegistryState},
        shm::{Shm, ShmHandler, raw::RawPool},
    },
    toplevel_info::{ToplevelInfo, ToplevelInfoHandler, ToplevelInfoState},
    toplevel_management::{ToplevelManagerHandler, ToplevelManagerState},
    wayland_client::{
        Connection, Proxy, QueueHandle, WEnum, delegate_noop,
        globals::registry_queue_init,
        protocol::{wl_buffer, wl_output, wl_shm},
    },
    wayland_protocols::ext::{
        foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1 as Toplevel,
        workspace::v1::client::ext_workspace_handle_v1::{
            self, ExtWorkspaceHandleV1 as WsHandle,
        },
    },
    workspace::{WorkspaceHandler, WorkspaceState},
};
use cosmic_client_toolkit::wayland_protocols::ext::workspace::v1::client::ext_workspace_group_handle_v1::ExtWorkspaceGroupHandleV1;

fn debug() -> bool {
    std::env::var_os("MAXWS_DEBUG").is_some()
}

macro_rules! log {
    ($($t:tt)*) => { if debug() { eprintln!($($t)*); } };
}

#[derive(Default)]
struct Tracked {
    big: bool,
    /// We just moved it; COSMIC drops the maximized state on a move, so we
    /// re-maximize it and ignore the brief "restored" state until it settles.
    settling: bool,
    /// Workspace it was last seen on, to notice when it gets moved.
    ws: Option<WsHandle>,
}

struct PendingNew {
    toplevel: Toplevel,
    group: ExtWorkspaceGroupHandleV1,
    before: HashSet<WsHandle>,
    output: wl_output::WlOutput,
    since: Instant,
}

#[derive(Clone, Debug, PartialEq)]
enum PanelMode {
    Wallpaper,
    App(String), // the background value written, always "Color((...))"
}

struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    screencopy_state: ScreencopyState,
    toplevel_info_state: ToplevelInfoState,
    toplevel_manager_state: ToplevelManagerState,
    workspace_state: WorkspaceState,
    tracked: HashMap<Toplevel, Tracked>,
    pin_requested: HashSet<WsHandle>,
    unpin_requested: HashSet<WsHandle>,
    /// Waiting for a workspace we asked COSMIC to create (Mac Spaces mode).
    pending_new: Option<PendingNew>,
    ready: bool,
    qh: QueueHandle<App>,
    /// Memory for the in-flight capture. Kept here, not in the frame's Wayland
    /// user data: freeing it sends a Wayland request, which deadlocks if it
    /// happens while the event queue is dispatching.
    capture_pool: Option<(RawPool, wl_buffer::WlBuffer)>,
    /// In-flight capture: (session, window, display name).
    capture: Option<(CaptureSession, Toplevel, String)>,
    /// Per display: last sampled window + title, to avoid re-sampling constantly.
    last_capture: HashMap<String, (Toplevel, String, Instant)>,
    /// Per display: what its top bar currently shows.
    panel_modes: HashMap<String, PanelMode>,
    /// Last focused app written for the top bar's app menu ("app_id\ntitle").
    focused: String,
    /// One-shot command from the top bar's app menu: (action, app_id).
    cli: Option<(String, String)>,
    cli_done: bool,
}

fn is_big(info: &ToplevelInfo) -> bool {
    (info.state.contains(&TState::Maximized) || info.state.contains(&TState::Fullscreen))
        && !info.state.contains(&TState::Minimized)
        && !info.state.contains(&TState::Sticky)
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

fn cosmic_dir(id: &str) -> String {
    format!("{}/.config/cosmic/{id}/v1", home())
}

/// Top bar entry name for a display, e.g. "Panel-eDP-1".
fn panel_entry(output: &str) -> String {
    format!("Panel-{output}")
}

fn panel_file(entry: &str, key: &str) -> String {
    format!("{}/{key}", cosmic_dir(&format!("com.system76.CosmicPanel.{entry}")))
}

fn write_if_changed(path: &str, value: &str) {
    if fs::read_to_string(path).map(|c| c.trim() == value).unwrap_or(false) {
        return;
    }
    let _ = fs::write(path, value);
}

/// Keep a bar color readable under the theme's text color without leaving "custom color" mode.
fn adjust_for_text(rgb: [f32; 3], lum: f32, dark_mode: bool) -> [f32; 3] {
    if dark_mode && lum > 0.72 {
        let k = 0.55 / lum;
        [rgb[0] * k, rgb[1] * k, rgb[2] * k]
    } else if !dark_mode && lum < 0.28 {
        let t = ((0.45 - lum) / (1.0 - lum)).clamp(0.0, 1.0);
        [rgb[0] + (1.0 - rgb[0]) * t, rgb[1] + (1.0 - rgb[1]) * t, rgb[2] + (1.0 - rgb[2]) * t]
    } else {
        rgb
    }
}

fn system_is_dark() -> bool {
    fs::read_to_string(format!(
        "{}/.config/cosmic/com.system76.CosmicTheme.Mode/v1/is_dark",
        home()
    ))
    .map(|s| s.trim() == "true")
    .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Workspaces
// ---------------------------------------------------------------------------

impl App {
    fn visible_toplevels_on<'a>(&'a self, ws: &'a WsHandle) -> impl Iterator<Item = &'a ToplevelInfo> {
        self.toplevel_info_state
            .toplevels()
            .filter(move |t| t.workspace.contains(ws))
            .filter(|t| !t.state.contains(&TState::Minimized) && !t.state.contains(&TState::Sticky))
    }

    fn others_on(&self, ws: &WsHandle, except: &Toplevel) -> usize {
        self.visible_toplevels_on(ws)
            .filter(|t| &t.foreign_toplevel != except)
            .count()
    }

    fn group_workspaces(&self, ws: &WsHandle) -> Vec<cosmic_client_toolkit::workspace::Workspace> {
        let Some(group) = self
            .workspace_state
            .workspace_groups()
            .find(|g| g.workspaces.contains(ws))
        else {
            return Vec::new();
        };
        let mut list: Vec<_> = group
            .workspaces
            .iter()
            .filter_map(|h| self.workspace_state.workspace_info(h).cloned())
            .collect();
        list.sort_by(|a, b| a.coordinates.cmp(&b.coordinates));
        list
    }

    /// Leftmost workspace of the display `ws` belongs to.
    fn desktop_of(&self, ws: &WsHandle) -> Option<WsHandle> {
        let list = self.group_workspaces(ws);
        list.iter()
            .find(|w| self.is_pinned(w))
            .or(list.first())
            .map(|w| w.handle.clone())
    }

    fn is_pinned(&self, w: &cosmic_client_toolkit::workspace::Workspace) -> bool {
        w.cosmic_state.contains(zcosmic_workspace_handle_v2::State::Pinned)
            || self.pin_requested.contains(&w.handle)
    }

    /// Rightmost workspace of the display (COSMIC keeps an empty one there).
    fn last_of(&self, ws: &WsHandle) -> Option<WsHandle> {
        self.group_workspaces(ws).last().map(|w| w.handle.clone())
    }

    fn is_active(&self, ws: &WsHandle) -> bool {
        self.workspace_state
            .workspace_info(ws)
            .is_some_and(|w| w.state.contains(ext_workspace_handle_v1::State::Active))
    }

    fn commit_workspaces(&self) {
        if let Ok(manager) = self.workspace_state.workspace_manager().get() {
            manager.commit();
        }
    }

    fn activate(&self, ws: &WsHandle) {
        ws.activate();
        self.commit_workspaces();
    }

    /// Keep exactly one desktop workspace per display, pinned:
    /// - no desktop yet: pin the first workspace without a maximized app;
    /// - a second desktop (dragged over from the other display): its normal
    ///   windows join this display's desktop and it stops being a desktop.
    fn enforce_desktops(&mut self) {
        let mut changed = false;
        let groups: Vec<_> = self.workspace_state.workspace_groups().cloned().collect();
        for group in groups {
            let mut list: Vec<_> = group
                .workspaces
                .iter()
                .filter_map(|h| self.workspace_state.workspace_info(h).cloned())
                .collect();
            list.sort_by(|a, b| a.coordinates.cmp(&b.coordinates));
            if list.is_empty() {
                continue;
            }
            let pinned: Vec<_> = list.iter().filter(|w| self.is_pinned(w)).cloned().collect();

            if pinned.is_empty() {
                let candidate = list
                    .iter()
                    .find(|w| !self.visible_toplevels_on(&w.handle).any(is_big))
                    .cloned();
                if let Some(w) = candidate {
                    if let Some(cosmic) = w.cosmic_handle.as_ref() {
                        log!("pinning desktop workspace {:?}", w.handle.id());
                        cosmic.pin();
                        self.pin_requested.insert(w.handle.clone());
                        changed = true;
                    }
                }
                continue;
            }

            // The desktop may sit anywhere in the row (you can drag workspaces to
            // its left); it is recognised by being pinned, not by position.
            let desktop = pinned[0].clone();

            for extra in &pinned[1..] {
                if self.unpin_requested.contains(&extra.handle) {
                    continue;
                }
                log!("second desktop on one display -> merging it");
                let output = group.outputs.first().cloned();
                let windows: Vec<_> = self
                    .visible_toplevels_on(&extra.handle)
                    .filter(|t| !is_big(t))
                    .cloned()
                    .collect();
                if let Some(output) = output {
                    for info in windows {
                        if let Some(cosmic) = info.cosmic_toplevel.as_ref() {
                            self.toplevel_manager_state
                                .manager
                                .move_to_ext_workspace(cosmic, &desktop.handle, &output);
                        }
                    }
                }
                if let Some(cosmic) = extra.cosmic_handle.as_ref() {
                    cosmic.unpin();
                }
                self.pin_requested.remove(&extra.handle);
                self.unpin_requested.insert(extra.handle.clone());
                changed = true;
            }
        }
        if changed {
            self.commit_workspaces();
        }
    }

    fn move_to(&self, info: &ToplevelInfo, ws: &WsHandle, output: &wl_output::WlOutput, remaximize: bool) {
        let Some(cosmic) = info.cosmic_toplevel.as_ref() else {
            return;
        };
        let manager = &self.toplevel_manager_state.manager;
        manager.move_to_ext_workspace(cosmic, ws, output);
        if remaximize {
            manager.set_maximized(cosmic);
        }
        self.activate(ws);
    }

    fn handle(&mut self, toplevel: &Toplevel) {
        let Some(info) = self.toplevel_info_state.info(toplevel).cloned() else {
            return;
        };
        let big = is_big(&info);
        let tracked = self.tracked.entry(toplevel.clone()).or_default();
        let was_big = tracked.big;
        let current_ws = info.workspace.iter().next().cloned();
        let prev_ws = std::mem::replace(&mut tracked.ws, current_ws.clone());

        if tracked.settling {
            if big {
                tracked.settling = false;
                tracked.big = true;
            }
            return;
        }
        tracked.big = big;

        if !self.ready {
            return;
        }
        if big == was_big {
            // Not a maximize/restore: check whether it was moved somewhere it
            // should not be (e.g. dragged to the other display).
            if let (Some(prev), Some(ws)) = (prev_ws, current_ws) {
                if prev != ws {
                    self.handle_moved(&info, &ws);
                }
            }
            return;
        }
        let (Some(ws), Some(output)) = (
            info.workspace.iter().next().cloned(),
            info.output.iter().next().cloned(),
        ) else {
            return;
        };
        log!("{:?} [{}] big {was_big} -> {big}", info.app_id, info.title);
        let desktop = self.desktop_of(&ws);

        if big {
            // Maximized: it gets its own workspace, unless it already has one
            // (alone on a workspace that is not the desktop).
            let on_desktop = desktop.as_ref() == Some(&ws);
            if !on_desktop && self.others_on(&ws, toplevel) == 0 {
                return;
            }
            log!("  -> own workspace on the right");
            self.give_own_workspace(&info, &ws, &output);
        } else {
            // Restored: back to the desktop workspace of this display.
            let Some(desktop) = desktop else {
                return;
            };
            if desktop != ws {
                log!("  -> back to desktop workspace");
                self.move_to(&info, &desktop, &output, false);
            }
        }
    }
}

impl App {
    fn remaximize_flag(info: &ToplevelInfo) -> bool {
        info.state.contains(&TState::Maximized) && !info.state.contains(&TState::Fullscreen)
    }

    /// Put a maximized window on a workspace of its own on this display.
    /// Uses COSMIC's spare empty workspace if there is one (stock COSMIC);
    /// otherwise asks COSMIC to create one (Mac Spaces mode) and moves the
    /// window once it appears.
    fn give_own_workspace(&mut self, info: &ToplevelInfo, ws: &WsHandle, output: &wl_output::WlOutput) {
        let toplevel = &info.foreign_toplevel;
        let desktop = self.desktop_of(ws);
        if let Some(last) = self.last_of(ws) {
            if &last != ws && desktop.as_ref() != Some(&last) && self.visible_toplevels_on(&last).next().is_none() {
                let remax = Self::remaximize_flag(info);
                if let Some(t) = self.tracked.get_mut(toplevel) {
                    t.settling = remax;
                }
                self.move_to(info, &last, output, remax);
                return;
            }
        }
        let Some(group) = self.workspace_state.workspace_groups().find(|g| g.workspaces.contains(ws)).cloned() else {
            return;
        };
        log!("  asking COSMIC for a new workspace");
        group.handle.create_workspace(String::new());
        self.commit_workspaces();
        self.pending_new = Some(PendingNew {
            toplevel: toplevel.clone(),
            group: group.handle.clone(),
            before: group.workspaces.clone(),
            output: output.clone(),
            since: Instant::now(),
        });
    }

    /// Called when COSMIC reports workspace changes: finish a pending move.
    fn finish_pending_new(&mut self) {
        let Some(p) = self.pending_new.as_ref() else {
            return;
        };
        if p.since.elapsed().as_secs() >= 3 {
            log!("  new workspace never appeared");
            self.pending_new = None;
            return;
        }
        let Some(group) = self.workspace_state.workspace_group_info(&p.group) else {
            return;
        };
        let Some(new_ws) = group.workspaces.iter().find(|w| !p.before.contains(*w)).cloned() else {
            return;
        };
        let p = self.pending_new.take().unwrap();
        let Some(info) = self.toplevel_info_state.info(&p.toplevel).cloned() else {
            return;
        };
        let remax = Self::remaximize_flag(&info);
        if let Some(t) = self.tracked.get_mut(&p.toplevel) {
            t.settling = remax;
        }
        log!("  -> moving into the new workspace");
        self.move_to(&info, &new_ws, &p.output, remax);
    }

    /// A window changed workspace (dragged to another display, or moved with a
    /// shortcut). Keep the rules: normal windows live on the desktop workspace,
    /// maximized windows each have a workspace of their own.
    fn handle_moved(&mut self, info: &ToplevelInfo, ws: &WsHandle) {
        let toplevel = &info.foreign_toplevel;
        let Some(output) = info.output.iter().next().cloned() else {
            return;
        };
        let Some(desktop) = self.desktop_of(ws) else {
            return;
        };
        if !is_big(info) {
            // Normal window landed in a maximized app's space: send it to that
            // display's desktop instead.
            let holds_big_app = self
                .visible_toplevels_on(ws)
                .any(|t| &t.foreign_toplevel != toplevel && is_big(t));
            if &desktop != ws && holds_big_app {
                log!("{:?} moved into a maximized app's space -> desktop", info.app_id);
                self.move_to(info, &desktop, &output, false);
            }
        } else {
            // Maximized window moved (e.g. to the other display): give it its
            // own workspace there if it is sharing one or sits on the desktop.
            if &desktop == ws || self.others_on(ws, toplevel) > 0 {
                log!("{:?} maximized window moved -> its own workspace", info.app_id);
                self.give_own_workspace(info, ws, &output);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Top bar color
// ---------------------------------------------------------------------------

struct SessionData {
    session_data: ScreencopySessionData,
}
impl ScreencopySessionDataExt for SessionData {
    fn screencopy_session_data(&self) -> &ScreencopySessionData {
        &self.session_data
    }
}

struct FrameData {
    frame_data: ScreencopyFrameData,
    size: (u32, u32),
    format: wl_shm::Format,
    output: String,
}
impl ScreencopyFrameDataExt for FrameData {
    fn screencopy_frame_data(&self) -> &ScreencopyFrameData {
        &self.frame_data
    }
}

impl App {
    fn output_name(&self, output: &wl_output::WlOutput) -> Option<String> {
        self.output_state.info(output).and_then(|i| i.name)
    }

    /// Make sure every display has its own top bar ("Panel-<display>") and dock
    /// ("Dock-<display>"), copied from the "Panel"/"Dock" templates, and that
    /// COSMIC's panel list uses them.
    fn ensure_panels(&mut self) {
        let names: Vec<String> = self.output_state.outputs().filter_map(|o| self.output_name(&o)).collect();
        if names.is_empty() {
            return;
        }
        let template = cosmic_dir("com.system76.CosmicPanel.Panel");
        let Ok(files) = fs::read_dir(&template) else {
            return;
        };
        let files: Vec<_> = files.filter_map(|f| f.ok()).collect();
        for name in &names {
            let entry = panel_entry(name);
            let dir = cosmic_dir(&format!("com.system76.CosmicPanel.{entry}"));
            let _ = fs::create_dir_all(&dir);
            for f in &files {
                let key = f.file_name().to_string_lossy().to_string();
                if matches!(key.as_str(), "output" | "name" | "background" | "opacity") {
                    continue;
                }
                if let Ok(v) = fs::read_to_string(f.path()) {
                    write_if_changed(&format!("{dir}/{key}"), v.trim());
                }
            }
            write_if_changed(&format!("{dir}/output"), &format!("Name(\"{name}\")"));
            write_if_changed(&format!("{dir}/name"), &format!("\"{entry}\""));
            for key in ["background", "opacity"] {
                if fs::metadata(format!("{dir}/{key}")).is_err() {
                    if let Ok(v) = fs::read_to_string(format!("{template}/{key}")) {
                        let _ = fs::write(format!("{dir}/{key}"), v.trim());
                    }
                }
            }
        }
        // Per-display docks ("Dock-<display>") from the "Dock" template, so each
        // screen's dock can hide on its own when that screen shows a maximized window.
        let dock_template = cosmic_dir("com.system76.CosmicPanel.Dock");
        if let Ok(dfiles) = fs::read_dir(&dock_template) {
            let dfiles: Vec<_> = dfiles.filter_map(|f| f.ok()).collect();
            for name in &names {
                let entry = format!("Dock-{name}");
                let dir = cosmic_dir(&format!("com.system76.CosmicPanel.{entry}"));
                let _ = fs::create_dir_all(&dir);
                for f in &dfiles {
                    let key = f.file_name().to_string_lossy().to_string();
                    if matches!(key.as_str(), "output" | "name" | "autohide") {
                        continue;
                    }
                    if let Ok(v) = fs::read_to_string(f.path()) {
                        write_if_changed(&format!("{dir}/{key}"), v.trim());
                    }
                }
                write_if_changed(&format!("{dir}/output"), &format!("Name(\"{name}\")"));
                write_if_changed(&format!("{dir}/name"), &format!("\"{entry}\""));
                if fs::metadata(format!("{dir}/autohide")).is_err() {
                    let _ = fs::write(format!("{dir}/autohide"), "Never");
                }
            }
        }
        // Panel list: per-display bars instead of the shared "Panel".
        let entries_path = format!("{}/entries", cosmic_dir("com.system76.CosmicPanel"));
        let current = fs::read_to_string(&entries_path).unwrap_or_default();
        let mut list: Vec<String> = current
            .split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_string)
            .filter(|e| e != "Panel")
            .collect();
        // "Dock" stays in the list so COSMIC Settings shows the Dock as on and edits
        // its settings; it is pointed at a display that doesn't exist, and the real
        // docks are the per-display copies. Turning the Dock off in Settings removes
        // "Dock", which removes the copies too (see also dock-sync.sh).
        let dock_on = list.iter().any(|e| e == "Dock");
        write_if_changed(
            &format!("{}/output", cosmic_dir("com.system76.CosmicPanel.Dock")),
            "Name(\"cosmic-mac-template\")",
        );
        list.retain(|e| dock_on || !e.starts_with("Dock-"));
        for name in &names {
            let entry = panel_entry(name);
            if !list.contains(&entry) {
                list.insert(0, entry);
            }
        }
        for name in names.iter().filter(|_| dock_on) {
            let entry = format!("Dock-{name}");
            if !list.contains(&entry) {
                list.push(entry);
            }
        }
        let ron = format!(
            "[\n{}\n]",
            list.iter().map(|e| format!("    \"{e}\",")).collect::<Vec<_>>().join("\n")
        );
        write_if_changed(&entries_path, &ron);

    }

    /// The maximized/fullscreen window shown on this display, if any
    /// (the focused one first).
    fn big_window_on(&self, output: &wl_output::WlOutput) -> Option<ToplevelInfo> {
        let group = self.workspace_state.workspace_groups().find(|g| g.outputs.contains(output))?;
        let active = group.workspaces.iter().find(|w| self.is_active(w))?;
        let mut big: Vec<_> = self.visible_toplevels_on(active).filter(|t| is_big(t)).cloned().collect();
        big.sort_by_key(|t| !t.state.contains(&TState::Activated));
        big.into_iter().next()
    }

    fn update_panels(&mut self) {
        if !self.ready {
            return;
        }
        let outputs: Vec<_> = self.output_state.outputs().collect();
        for output in outputs {
            let Some(name) = self.output_name(&output) else {
                log!("display without a name, skipped");
                continue;
            };
            let big = self.big_window_on(&output);
            log!("{name}: showing {:?}", big.as_ref().map(|i| &i.app_id));
            write_if_changed(
                &panel_file(&format!("Dock-{name}"), "autohide"),
                if big.is_some() { "Always" } else { "Never" },
            );
            match big {
                Some(info) => {
                    if let Some((t, title, when)) = self.last_capture.get(&name) {
                        if *t == info.foreign_toplevel
                            && *title == info.title
                            && when.elapsed().as_secs() < 30
                            && matches!(self.panel_modes.get(&name), Some(PanelMode::App(_)))
                        {
                            continue;
                        }
                    }
                    if self.capture.is_some() {
                        log!("{name}: waiting for another capture");
                        continue; // one at a time; update_panels runs again when it finishes
                    }
                    let source = CaptureSource::Toplevel(info.foreign_toplevel.clone());
                    match self.screencopy_state.capturer().create_session(
                        &source,
                        CaptureOptions::empty(),
                        &self.qh,
                        SessionData { session_data: ScreencopySessionData::default() },
                    ) {
                        Ok(session) => {
                            self.last_capture
                                .insert(name.clone(), (info.foreign_toplevel.clone(), info.title.clone(), Instant::now()));
                            self.capture = Some((session, info.foreign_toplevel.clone(), name));
                        }
                        Err(e) => log!("capture unavailable: {e:?}"),
                    }
                }
                None => {
                    // While a maximized app is being moved to its new workspace,
                    // that workspace is briefly empty: don't flash the bar.
                    let moving = self.pending_new.is_some() || self.tracked.values().any(|t| t.settling);
                    if !moving {
                        self.set_wallpaper_mode(&name);
                    }
                }
            }
        }
        self.save_modes();
    }

    fn save_modes(&self) {
        let text: String = self
            .panel_modes
            .iter()
            .map(|(name, mode)| {
                let m = if matches!(mode, PanelMode::Wallpaper) { "wallpaper" } else { "app" };
                format!("{} {} {}\n", panel_entry(name), name, m)
            })
            .collect();
        let dir = format!("{}/.cache/cosmic-mac", home());
        let _ = fs::create_dir_all(&dir);
        write_if_changed(&format!("{dir}/panel-modes"), text.trim_end());
    }

    fn set_wallpaper_mode(&mut self, name: &str) {
        self.last_capture.remove(name);
        if self.panel_modes.get(name) == Some(&PanelMode::Wallpaper) {
            return;
        }
        log!("{name}: top bar -> wallpaper glass");
        let _ = std::process::Command::new(format!("{}/.local/share/cosmic-mac/panel-contrast.py", home()))
            .args(["--output", name, "--panel", &panel_entry(name)])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        self.panel_modes.insert(name.to_string(), PanelMode::Wallpaper);
    }

    fn set_app_color(&mut self, name: &str, rgb: [f32; 3]) {
        let lum = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
        // A custom color keeps the system theme's text color (light text in
        // dark mode), so use the app's exact color unless it clearly clashes.
        let dark_mode = system_is_dark();
        // Always a custom color: switching between a custom color and the Dark/Light
        // styles makes cosmic-panel rebuild the whole bar (every applet reloads, which
        // flashes the bar on each workspace change). So a color that would clash with the
        // theme's text is darkened (dark theme) or lightened (light theme) instead.
        let rgb = adjust_for_text(rgb, lum, dark_mode);
        let value = format!("Color(({:.3}, {:.3}, {:.3}))", rgb[0], rgb[1], rgb[2]);
        if self.panel_modes.get(name) == Some(&PanelMode::App(value.clone())) {
            return;
        }
        log!("{name}: top bar -> app color {rgb:?} => {value}");
        let entry = panel_entry(name);
        write_if_changed(&panel_file(&entry, "opacity"), "0.85");
        write_if_changed(&panel_file(&entry, "background"), &value);
        self.panel_modes.insert(name.to_string(), PanelMode::App(value));
    }
}

impl App {
    fn release_capture_pool(&mut self) {
        if let Some((pool, buffer)) = self.capture_pool.take() {
            buffer.destroy();
            drop(pool);
        }
    }
}

impl ScreencopyHandler for App {
    fn screencopy_state(&mut self) -> &mut ScreencopyState {
        &mut self.screencopy_state
    }

    fn init_done(&mut self, _: &Connection, qh: &QueueHandle<Self>, session: &CaptureSession, formats: &Formats) {
        let Some(output) = self.capture.as_ref().map(|(_, _, o)| o.clone()) else {
            return;
        };
        let (w, h) = formats.buffer_size;
        let preferred = [
            wl_shm::Format::Abgr8888,
            wl_shm::Format::Xbgr8888,
            wl_shm::Format::Argb8888,
            wl_shm::Format::Xrgb8888,
        ];
        let Some(format) = preferred.into_iter().find(|f| formats.shm_formats.contains(f)) else {
            self.capture = None;
            return;
        };
        let Ok(mut pool) = RawPool::new(w as usize * h as usize * 4, &self.shm) else {
            self.capture = None;
            return;
        };
        let buffer = pool.create_buffer(0, w as i32, h as i32, w as i32 * 4, format, (), qh);
        session.capture(
            &buffer,
            &[],
            qh,
            FrameData { frame_data: ScreencopyFrameData::default(), size: (w, h), format, output },
        );
        self.release_capture_pool();
        self.capture_pool = Some((pool, buffer));
    }

    fn stopped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &CaptureSession) {
        log!("capture session stopped");
        self.capture = None;
    }

    fn ready(&mut self, _: &Connection, _: &QueueHandle<Self>, frame: &CaptureFrame, _: Frame) {
        let output = frame.data::<FrameData>().map(|d| d.output.clone()).unwrap_or_default();
        let rgb = {
            let data = frame.data::<FrameData>().unwrap();
            let Some((pool, _)) = self.capture_pool.as_mut() else {
                self.capture = None;
                return;
            };
            let bytes = pool.mmap();
            let (w, h) = data.size;
            // Sample the window's top band (its header / tab bar), skipping the edge.
            let (y0, y1) = (2.min(h), 36.min(h));
            let bgr = matches!(data.format, wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888);
            let (mut sum, mut n) = ([0f64; 3], 0f64);
            for y in y0..y1 {
                for x in (8..w.saturating_sub(8)).step_by(4) {
                    let i = ((y * w + x) * 4) as usize;
                    let px = &bytes[i..i + 4];
                    if px[3] == 0 && matches!(data.format, wl_shm::Format::Abgr8888 | wl_shm::Format::Argb8888) {
                        continue;
                    }
                    let (r, g, b) = if bgr { (px[2], px[1], px[0]) } else { (px[0], px[1], px[2]) };
                    sum[0] += r as f64;
                    sum[1] += g as f64;
                    sum[2] += b as f64;
                    n += 1.0;
                }
            }
            (n > 0.0).then(|| [(sum[0] / n / 255.0) as f32, (sum[1] / n / 255.0) as f32, (sum[2] / n / 255.0) as f32])
        };
        self.capture = None;
        self.release_capture_pool();
        if let Some(rgb) = rgb {
            self.set_app_color(&output, rgb);
        }
        // Other displays may be waiting, or focus changed while capturing.
        self.update_panels();
    }

    fn failed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &CaptureFrame, reason: WEnum<FailureReason>) {
        log!("capture failed: {reason:?}");
        self.capture = None;
        self.release_capture_pool();
        self.update_panels();
    }
}

// ---------------------------------------------------------------------------
// Wayland plumbing
// ---------------------------------------------------------------------------

impl App {
    /// Tell the top bar's app menu which app is focused (only while one is).
    fn write_focused(&mut self) {
        let Some(info) = self
            .toplevel_info_state
            .toplevels()
            .find(|i| i.state.contains(&TState::Activated))
        else {
            return;
        };
        let now = format!("{}\n{}\n", info.app_id, info.title);
        if now != self.focused {
            let dir = format!("{}/cosmic-mac", std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into()));
            let _ = fs::create_dir_all(&dir);
            let _ = fs::write(format!("{dir}/focused.tmp"), &now);
            let _ = fs::rename(format!("{dir}/focused.tmp"), format!("{dir}/focused"));
            self.focused = now;
        }
    }

    /// `--app quit|hide|hide-others <app_id>` from the top bar's app menu.
    fn run_cli(&mut self) {
        let Some((action, app_id)) = self.cli.clone() else { return };
        let manager = &self.toplevel_manager_state.manager;
        for info in self.toplevel_info_state.toplevels() {
            let Some(cosmic) = info.cosmic_toplevel.as_ref() else { continue };
            let mine = info.app_id == app_id;
            match action.as_str() {
                "quit" if mine => manager.close(cosmic),
                "hide" if mine => manager.set_minimized(cosmic),
                "hide-others" if !mine && !info.state.contains(&TState::Sticky) => manager.set_minimized(cosmic),
                _ => {}
            }
        }
        self.cli_done = true;
    }
}

impl ToplevelInfoHandler for App {
    fn toplevel_info_state(&mut self) -> &mut ToplevelInfoState {
        &mut self.toplevel_info_state
    }

    fn new_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, toplevel: &Toplevel) {
        if self.cli.is_some() {
            return;
        }
        self.handle(toplevel);
        self.update_panels();
        self.write_focused();
    }

    fn update_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, toplevel: &Toplevel) {
        if self.cli.is_some() {
            return;
        }
        self.handle(toplevel);
        self.update_panels();
        self.write_focused();
    }

    fn toplevel_closed(&mut self, _: &Connection, _: &QueueHandle<Self>, toplevel: &Toplevel) {
        if self.cli.is_some() {
            return;
        }
        let tracked = self.tracked.remove(toplevel);
        if let Some(info) = self.toplevel_info_state.info(toplevel).cloned() {
            // Closing a full-screen app returns to that display's desktop, like macOS.
            if tracked.is_some_and(|t| t.big) {
                if let Some(ws) = info.workspace.iter().next() {
                    if self.is_active(ws) && self.others_on(ws, toplevel) == 0 {
                        if let Some(desktop) = self.desktop_of(ws) {
                            if &desktop != ws {
                                self.activate(&desktop);
                            }
                        }
                    }
                }
            }
        }
        if self.capture.as_ref().is_some_and(|(_, t, _)| t == toplevel) {
            self.capture = None;
        }
        self.update_panels();
    }

    fn info_done(&mut self, _: &Connection, _: &QueueHandle<Self>) {
        if self.cli.is_some() {
            return self.run_cli();
        }
        if !self.ready {
            self.ready = true;
            self.ensure_panels();
            self.enforce_desktops();
            self.update_panels();
        }
    }
}

impl ToplevelManagerHandler for App {
    fn toplevel_manager_state(&mut self) -> &mut ToplevelManagerState {
        &mut self.toplevel_manager_state
    }
    fn capabilities(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: Vec<WEnum<zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1>>,
    ) {
    }
}

impl WorkspaceHandler for App {
    fn workspace_state(&mut self) -> &mut WorkspaceState {
        &mut self.workspace_state
    }
    fn done(&mut self) {
        if self.ready {
            self.finish_pending_new();
            self.enforce_desktops();
            // Switching workspaces changes what each display shows.
            self.update_panels();
        }
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        if self.ready {
            self.ensure_panels();
            self.update_panels();
        }
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        if self.ready {
            self.ensure_panels();
        }
    }
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    sctk::registry_handlers!(OutputState);
}

fn main() {
    let conn = Connection::connect_to_env().expect("not running under a Wayland compositor");
    let (globals, mut queue) = registry_queue_init(&conn).expect("failed to read Wayland globals");
    let qh = queue.handle();
    let registry_state = RegistryState::new(&globals);

    let mut app = App {
        output_state: OutputState::new(&globals, &qh),
        shm: Shm::bind(&globals, &qh).expect("wl_shm not available"),
        screencopy_state: ScreencopyState::new(&globals, &qh),
        toplevel_info_state: ToplevelInfoState::try_new(&registry_state, &qh)
            .expect("compositor lacks toplevel info (not COSMIC?)"),
        toplevel_manager_state: ToplevelManagerState::try_new(&registry_state, &qh)
            .expect("compositor lacks toplevel management (not COSMIC?)"),
        workspace_state: WorkspaceState::new(&registry_state, &qh),
        registry_state,
        tracked: HashMap::new(),
        pin_requested: HashSet::new(),
        unpin_requested: HashSet::new(),
        pending_new: None,
        ready: false,
        qh: qh.clone(),
        capture_pool: None,
        capture: None,
        last_capture: HashMap::new(),
        panel_modes: HashMap::new(),
        focused: String::new(),
        cli: None,
        cli_done: false,
    };
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 4 && args[1] == "--app" {
        app.cli = Some((args[2].clone(), args[3].clone()));
    }

    loop {
        queue.blocking_dispatch(&mut app).expect("Wayland connection lost");
        if app.cli_done {
            let _ = conn.flush();
            let _ = queue.roundtrip(&mut app);
            break;
        }
    }
}

sctk::delegate_output!(App);
sctk::delegate_registry!(App);
sctk::delegate_shm!(App);
cosmic_client_toolkit::delegate_toplevel_info!(App);
cosmic_client_toolkit::delegate_toplevel_manager!(App);
cosmic_client_toolkit::delegate_workspace!(App);
cosmic_client_toolkit::delegate_screencopy!(App);
delegate_noop!(App: ignore wl_buffer::WlBuffer);
