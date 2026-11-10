//! Auto-quit idle apps, to save battery: an app none of whose windows has been focused for
//! MINUTES (default 60) is asked to close, like clicking its close button (the app can still
//! ask to save; nothing is ever killed). By default only while running on battery.
//!
//! Settings: ~/.config/cosmic-mac/auto-quit.conf (created on first run, re-read every check):
//!   MINUTES=60
//!   ONLY_ON_BATTERY=1
//!   EXCLUDE=com.slack.Slack firefox* ...   app ids never closed (* matches anything, any case)
//! Apps playing sound (PipeWire output streams, via pw-dump) are skipped too.
//! Runs as the systemd user service cosmic-mac-autoquit.service; what it closes is logged
//! to the journal:  journalctl --user -u cosmic-mac-autoquit
//!
//!   cosmic-mac-autoquit --list   print the open apps and how long since each was focused

use std::{
    collections::HashMap,
    os::fd::AsRawFd,
    time::{Duration, Instant},
};

use cosmic_client_toolkit::{
    cosmic_protocols::{
        toplevel_info::v1::client::zcosmic_toplevel_handle_v1::State as TState,
        toplevel_management::v1::client::zcosmic_toplevel_manager_v1,
    },
    sctk::{
        self,
        output::{OutputHandler, OutputState},
        registry::{ProvidesRegistryState, RegistryState},
    },
    toplevel_info::{ToplevelInfoHandler, ToplevelInfoState},
    toplevel_management::{ToplevelManagerHandler, ToplevelManagerState},
    workspace::{WorkspaceHandler, WorkspaceState},
    wayland_client::{Connection, EventQueue, QueueHandle, WEnum, backend::WaylandError, globals::registry_queue_init},
    wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1 as Toplevel,
};

const DEFAULT_CONF: &str = "# Auto-quit: apps none of whose windows was focused for MINUTES get a normal close
# request (like clicking the close button; unsaved work still asks). See journalctl --user -u cosmic-mac-autoquit
MINUTES=60
# 1 = only while on battery, 0 = always
ONLY_ON_BATTERY=1
# App ids that are never closed (space separated, * matches anything, any case).
# Find an app's id with:  cosmic-mac-autoquit --list
EXCLUDE=com.slack.Slack Slack firefox* org.mozilla.firefox* FFPWA-* org.ferdium.Ferdium ferdium* *whatsapp* com.anthropic.Claude claude-desktop* com.system76.CosmicTerm org.gnome.Terminal org.gnome.Ptyxis org.gnome.Console kitty Alacritty org.wezfurlong.wezterm foot* konsole* com.mitchellh.ghostty xterm com.system76.CosmicEdit org.gnome.TextEditor gedit code code-oss code-url-handler com.visualstudio.code* codium* VSCodium jetbrains-* dev.zed.Zed* sublime_text cursor* com.system76.CosmicPlayer org.gnome.Totem vlc mpv spotify com.spotify.Client rhythmbox org.gnome.Rhythmbox3 org.gnome.Music io.github.celluloid_player.Celluloid com.system76.CosmicPanel* com.system76.CosmicLauncher com.system76.CosmicAppLibrary com.system76.CosmicWorkspaces com.system76.CosmicOsd com.system76.CosmicFiles org.gnome.Nautilus nautilus
";

fn conf_path() -> String {
    format!("{}/.config/cosmic-mac/auto-quit.conf", std::env::var("HOME").unwrap_or_default())
}

struct Settings {
    minutes: u64,
    only_on_battery: bool,
    exclude: Vec<String>,
}

fn load_settings() -> Settings {
    let path = conf_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        let _ = std::fs::create_dir_all(std::path::Path::new(&path).parent().unwrap());
        let _ = std::fs::write(&path, DEFAULT_CONF);
        DEFAULT_CONF.to_string()
    });
    let get = |key: &str| text.lines().find_map(|l| l.trim().strip_prefix(key)?.trim().strip_prefix('=').map(str::trim));
    Settings {
        minutes: get("MINUTES").and_then(|v| v.parse().ok()).filter(|m| *m > 0).unwrap_or(60),
        only_on_battery: get("ONLY_ON_BATTERY").is_none_or(|v| v != "0"),
        exclude: get("EXCLUDE")
            .unwrap_or("")
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|p| !p.is_empty())
            .map(str::to_lowercase)
            .collect(),
    }
}

/// `pattern` with `*` wildcards, case-insensitive (`pattern` is already lowercase).
fn glob(pattern: &str, text: &str) -> bool {
    let text = text.to_lowercase();
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut rest = text.as_str();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(part) else { return false };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            let Some(at) = rest.find(part) else { return false };
            rest = &rest[at + part.len()..];
        }
    }
    true
}

/// True while no mains adapter is online.
fn on_battery() -> bool {
    let Ok(dir) = std::fs::read_dir("/sys/class/power_supply") else { return false };
    let mut saw_mains = false;
    for e in dir.flatten() {
        let p = e.path();
        if std::fs::read_to_string(p.join("type")).is_ok_and(|t| t.trim() == "Mains") {
            saw_mains = true;
            if std::fs::read_to_string(p.join("online")).is_ok_and(|o| o.trim() == "1") {
                return false;
            }
        }
    }
    saw_mains
}

/// Names of the apps with a running PipeWire audio output stream (lowercase): their
/// application name, binary, Flatpak/portal id and application id.
fn playing_audio() -> Vec<String> {
    let Ok(out) = std::process::Command::new("pw-dump").output() else { return Vec::new() };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else { return Vec::new() };
    let mut names = Vec::new();
    for obj in json.as_array().into_iter().flatten() {
        let info = &obj["info"];
        let props = &info["props"];
        if props["media.class"].as_str() != Some("Stream/Output/Audio") || info["state"].as_str() != Some("running") {
            continue;
        }
        for key in ["application.name", "application.process.binary", "pipewire.access.portal.app_id", "application.id"] {
            if let Some(v) = props[key].as_str().filter(|v| !v.is_empty()) {
                names.push(v.to_lowercase());
            }
        }
    }
    names
}

fn plays(app_id: &str, audio: &[String]) -> bool {
    let id = app_id.to_lowercase();
    let last = id.rsplit('.').next().unwrap_or(&id).to_string();
    audio.iter().any(|n| *n == id || *n == last || (n.len() >= 4 && id.contains(n.as_str())))
}

struct App {
    registry_state: RegistryState,
    /// windows report the displays they are on
    output_state: OutputState,
    toplevel_info_state: ToplevelInfoState,
    toplevel_manager_state: ToplevelManagerState,
    /// bound because COSMIC reports a window's workspace with its info
    workspace_state: WorkspaceState,
    /// per app id: when one of its windows was last focused (or first seen)
    last_focus: HashMap<String, Instant>,
    /// the compositor has sent the first complete window list
    ready: bool,
}

impl App {
    fn note_focus(&mut self) {
        let now = Instant::now();
        let mut seen = Vec::new();
        for info in self.toplevel_info_state.toplevels() {
            seen.push(info.app_id.clone());
            if info.state.contains(&TState::Activated) {
                self.last_focus.insert(info.app_id.clone(), now);
            } else {
                self.last_focus.entry(info.app_id.clone()).or_insert(now);
            }
        }
        // forget apps that have no windows any more (they start fresh when reopened)
        self.last_focus.retain(|id, _| seen.contains(id));
    }

    fn check(&mut self) {
        self.note_focus();
        let s = load_settings();
        if s.only_on_battery && !on_battery() {
            // on mains power idle time doesn't count: start over when unplugged
            let now = Instant::now();
            self.last_focus.values_mut().for_each(|t| *t = now);
            return;
        }
        let limit = Duration::from_secs(s.minutes * 60);
        let idle: Vec<String> = self
            .last_focus
            .iter()
            .filter(|(id, t)| t.elapsed() >= limit && !id.is_empty() && !s.exclude.iter().any(|p| glob(p, id)))
            .map(|(id, _)| id.clone())
            .collect();
        if idle.is_empty() {
            return;
        }
        let audio = playing_audio();
        let manager = &self.toplevel_manager_state.manager;
        for id in idle {
            if plays(&id, &audio) {
                continue;
            }
            let mut windows = 0;
            for info in self.toplevel_info_state.toplevels().filter(|i| i.app_id == id) {
                if let Some(cosmic) = info.cosmic_toplevel.as_ref() {
                    manager.close(cosmic);
                    windows += 1;
                }
            }
            if windows > 0 {
                eprintln!("closed {id} ({windows} window{}): not focused for {} min", if windows == 1 { "" } else { "s" }, s.minutes);
            }
            // if it stays open (e.g. it asks to save), wait another full period before asking again
            self.last_focus.insert(id, Instant::now());
        }
    }
}

impl ToplevelInfoHandler for App {
    fn toplevel_info_state(&mut self) -> &mut ToplevelInfoState {
        &mut self.toplevel_info_state
    }
    fn new_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Toplevel) {
        self.note_focus();
    }
    fn update_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Toplevel) {
        self.note_focus();
    }
    fn toplevel_closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Toplevel) {}
    fn info_done(&mut self, _: &Connection, _: &QueueHandle<Self>) {
        self.ready = true;
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

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: sctk::reexports::client::protocol::wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: sctk::reexports::client::protocol::wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: sctk::reexports::client::protocol::wl_output::WlOutput) {}
}

impl WorkspaceHandler for App {
    fn workspace_state(&mut self) -> &mut WorkspaceState {
        &mut self.workspace_state
    }
    fn done(&mut self) {}
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
        toplevel_info_state: ToplevelInfoState::try_new(&registry_state, &qh).expect("compositor lacks toplevel info (not COSMIC?)"),
        toplevel_manager_state: ToplevelManagerState::try_new(&registry_state, &qh)
            .expect("compositor lacks toplevel management (not COSMIC?)"),
        workspace_state: WorkspaceState::new(&registry_state, &qh),
        registry_state,
        last_focus: HashMap::new(),
        ready: false,
    };
    // wait for the first complete window list (COSMIC sends it with its next frame)
    let give_up = Instant::now() + Duration::from_secs(3);
    while !app.ready && Instant::now() < give_up {
        dispatch(&conn, &mut queue, &mut app, give_up);
    }

    if std::env::args().nth(1).as_deref() == Some("--list") {
        let s = load_settings();
        let audio = playing_audio();
        let mut ids: Vec<String> = app.toplevel_info_state.toplevels().map(|i| i.app_id.clone()).collect();
        ids.sort();
        ids.dedup();
        for id in ids {
            let note = if s.exclude.iter().any(|p| glob(p, &id)) {
                "  (excluded)"
            } else if plays(&id, &audio) {
                "  (playing audio)"
            } else {
                ""
            };
            println!("{id}{note}");
        }
        return;
    }

    let s = load_settings();
    eprintln!(
        "watching: apps unfocused for {} min are closed{}",
        s.minutes,
        if s.only_on_battery { " (only on battery)" } else { "" }
    );
    let mut next_check = Instant::now();
    loop {
        if Instant::now() >= next_check {
            app.check();
            // check four times per period, at most once a minute
            let every = (load_settings().minutes * 15).clamp(15, 60);
            next_check = Instant::now() + Duration::from_secs(every);
        }
        dispatch(&conn, &mut queue, &mut app, next_check);
    }
}

/// Handles Wayland events until `until` (or until some arrive).
fn dispatch(conn: &Connection, queue: &mut EventQueue<App>, app: &mut App, until: Instant) {
    queue.dispatch_pending(app).expect("Wayland connection lost");
    let _ = conn.flush();
    if let Some(guard) = queue.prepare_read() {
        let mut fd = libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let wait = until.saturating_duration_since(Instant::now()).as_millis().min(i32::MAX as u128) as i32;
        if unsafe { libc::poll(&mut fd, 1, wait) } > 0 {
            match guard.read() {
                Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                r => {
                    r.expect("Wayland connection lost");
                }
            }
        }
    }
    queue.dispatch_pending(app).expect("Wayland connection lost");
}

sctk::delegate_output!(App);
sctk::delegate_registry!(App);
cosmic_client_toolkit::delegate_toplevel_info!(App);
cosmic_client_toolkit::delegate_toplevel_manager!(App);
cosmic_client_toolkit::delegate_workspace!(App);
