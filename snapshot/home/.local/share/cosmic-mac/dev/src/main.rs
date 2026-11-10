//! Developer services menu for the COSMIC panel (databases, web servers…).
//!
//! Panel: code-brackets glyph, filled while any of the services runs.
//! Popover (macOS menu styling): "Developer Services" switch (all on / all off), one switch
//! per service, "Edit Services…".
//! Services: ~/.config/cosmic-mac/dev-services, one per line ("unit Label"; "--user unit Label"
//! for a user unit). System units start and stop without a password once
//! power/setup-root.sh has installed its polkit rule (until then COSMIC asks for it).
//! Status from `systemctl is-active`: every 2 s while the menu is open, every 30 s otherwise,
//! and the bar is only redrawn when something changed.

#[path = "../../shared/style.rs"]
mod style;
#[path = "../../shared/watch.rs"]
mod watch;

use std::time::Duration;

use cosmic::iced::futures::StreamExt;

use cosmic::{
    Element, Task, app,
    iced::{Alignment, Length, Subscription, window},
    surface,
    theme::Button,
    widget::{Column, button, container, row, text, toggler},
};

const APP_ID: &str = "com.aqib.CosmicMacDev";

const DEFAULT_CONF: &str = "# Developer services in the top bar's Developer menu, one per line:  unit  Label
# Put --user in front of a unit that runs as you (systemctl --user).
mariadb.service MariaDB
postgresql.service PostgreSQL
nginx.service nginx
php-fpm.service PHP-FPM
supervisord.service Supervisor
--user supervisor-studio.service Supervisor Studio
";

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<Dev>(())
}

fn conf_path() -> String {
    format!("{}/.config/cosmic-mac/dev-services", std::env::var("HOME").unwrap_or_default())
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Service {
    unit: String,
    label: String,
    user: bool,
}

fn load_services() -> Vec<Service> {
    let path = conf_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        let _ = std::fs::write(&path, DEFAULT_CONF);
        DEFAULT_CONF.to_string()
    });
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let (user, l) = match l.strip_prefix("--user") {
                Some(rest) => (true, rest.trim_start()),
                None => (false, l),
            };
            let (unit, label) = l.split_once(char::is_whitespace).unwrap_or((l, ""));
            let unit = if unit.contains('.') { unit.to_string() } else { format!("{unit}.service") };
            let label = if label.trim().is_empty() { unit.trim_end_matches(".service").to_string() } else { label.trim().to_string() };
            Some(Service { unit, label, user })
        })
        .collect()
}

/// `systemctl is-active` for every service, in order ("active", "inactive", "failed"…).
async fn gather(services: Vec<Service>) -> Vec<String> {
    let mut out = vec![String::new(); services.len()];
    for user in [false, true] {
        let idx: Vec<usize> = (0..services.len()).filter(|&i| services[i].user == user).collect();
        if idx.is_empty() {
            continue;
        }
        let mut cmd = tokio::process::Command::new("systemctl");
        if user {
            cmd.arg("--user");
        }
        cmd.arg("is-active").args(idx.iter().map(|&i| services[i].unit.as_str()));
        let text = cmd.output().await.map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        for (&i, state) in idx.iter().zip(text.lines()) {
            out[i] = state.trim().to_string();
        }
    }
    out
}

/// `systemctl start|stop` the given services (system ones may ask COSMIC for a password
/// until the polkit rule is installed). Returns an error message when one failed.
async fn act(verb: &'static str, services: Vec<Service>) -> Option<String> {
    let mut errors = Vec::new();
    for user in [false, true] {
        let units: Vec<&str> = services.iter().filter(|s| s.user == user).map(|s| s.unit.as_str()).collect();
        if units.is_empty() {
            continue;
        }
        let mut cmd = tokio::process::Command::new("systemctl");
        if user {
            cmd.arg("--user");
        }
        if let Ok(o) = cmd.arg(verb).args(&units).output().await {
            if !o.status.success() {
                let msg = String::from_utf8_lossy(&o.stderr).lines().next().unwrap_or("").trim().to_string();
                errors.push(if msg.is_empty() { format!("Could not {verb} {}", units.join(", ")) } else { msg });
            }
        }
    }
    (!errors.is_empty()).then(|| errors.join("\n"))
}

fn running(state: &str) -> bool {
    matches!(state, "active" | "activating" | "reloading" | "deactivating")
}

#[derive(Default)]
struct Dev {
    core: app::Core,
    popup: Option<window::Id>,
    services: Vec<Service>,
    states: Vec<String>,
    /// units being started or stopped, until their state changes
    busy: Vec<(String, &'static str)>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
enum Message {
    TogglePopup,
    CloseRequested(window::Id),
    OpenRequest,
    States(Vec<Service>, Vec<String>),
    Set(usize, bool),
    SetAll(bool),
    Done(Option<String>),
    Edit,
}

impl Dev {
    fn any_running(&self) -> bool {
        self.states.iter().any(|s| running(s))
    }

    fn refresh(&self) -> app::Task<Message> {
        let services = self.services.clone();
        cosmic::task::future(async move { cosmic::Action::App(Message::States(services.clone(), gather(services).await)) })
    }

    fn run(&mut self, verb: &'static str, services: Vec<Service>) -> app::Task<Message> {
        self.error = None;
        for s in &services {
            self.busy.retain(|(u, _)| u != &s.unit);
            self.busy.push((s.unit.clone(), if verb == "start" { "Starting…" } else { "Stopping…" }));
        }
        let all = self.services.clone();
        cosmic::task::future(async move { cosmic::Action::App(Message::Done(act(verb, services).await)) })
            .chain(cosmic::task::future(async move { cosmic::Action::App(Message::States(all.clone(), gather(all).await)) }))
    }

    fn close_popup(&mut self) -> app::Task<Message> {
        match self.popup.take() {
            Some(p) => surface::surface_task(surface::action::destroy_popup(p)),
            None => Task::none(),
        }
    }
}

impl cosmic::Application for Dev {
    type Message = Message;
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    const APP_ID: &'static str = APP_ID;

    fn init(core: app::Core, _flags: ()) -> (Self, app::Task<Message>) {
        let app = Self { core, services: load_services(), ..Default::default() };
        let t = app.refresh();
        (app, t)
    }

    fn core(&self) -> &app::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut app::Core {
        &mut self.core
    }

    fn update(&mut self, message: Message) -> app::Task<Message> {
        match message {
            Message::TogglePopup => {
                if self.popup.is_some() {
                    return self.close_popup();
                }
                self.services = load_services();
                self.error = None;
                let new_id = window::Id::unique();
                let open = surface::surface_task(surface::action::app_popup(
                    |_| Default::default(),
                    move |app: &mut Dev| {
                        app.popup.replace(new_id);
                        app.core.applet.get_popup_settings(app.core.main_window_id().unwrap(), new_id, Some((1, 1)), None, None)
                    },
                    None,
                )).chain(style::blur(new_id)).chain(style::blur_soon(new_id, 150)).chain(style::blur_soon(new_id, 700));
                return Task::batch([open, self.refresh()]);
            }
            Message::CloseRequested(id) => {
                if Some(id) == self.popup {
                    self.popup = None;
                }
            }
            Message::OpenRequest => return self.update(Message::TogglePopup),
            Message::States(services, states) => {
                // ignore answers for a service list that has since been edited
                if services == self.services {
                    let old = std::mem::replace(&mut self.states, states);
                    let now = &self.states;
                    let changed: Vec<String> = self
                        .services
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| old.get(*i) != now.get(*i))
                        .map(|(_, s)| s.unit.clone())
                        .collect();
                    self.busy.retain(|(u, _)| !changed.contains(u));
                }
            }
            Message::Set(i, on) => {
                if let Some(s) = self.services.get(i).cloned() {
                    return self.run(if on { "start" } else { "stop" }, vec![s]);
                }
            }
            Message::SetAll(on) => {
                let list: Vec<Service> = self
                    .services
                    .iter()
                    .zip(self.states.iter())
                    .filter(|(_, st)| running(st) != on)
                    .map(|(s, _)| s.clone())
                    .collect();
                return self.run(if on { "start" } else { "stop" }, list);
            }
            Message::Done(err) => {
                if err.is_some() {
                    self.busy.clear();
                }
                self.error = err;
            }
            Message::Edit => {
                let _ = load_services(); // make sure the file exists
                let path = conf_path();
                std::thread::spawn(move || {
                    let _ = std::process::Command::new("cosmic-edit").arg(path).status();
                });
                return self.close_popup();
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let glyph = if self.any_running() { "com.aqib.dev-active-symbolic" } else { "com.aqib.dev-symbolic" };
        self.core.applet.icon_button(glyph).padding(style::bar_padding(&self.core.applet)).on_press_down(Message::TogglePopup).into()
    }

    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let item = |c: Element<'static, Message>| -> Element<'static, Message> { container(c).padding([4, 14]).width(Length::Fill).into() };
        let mut col = Column::new().push(item(
            row![text::heading("Developer Services").size(15).width(Length::Fill), toggler(self.any_running()).on_toggle(Message::SetAll)]
                .align_y(Alignment::Center)
                .into(),
        ));
        col = col.push(style::separator());
        if self.services.is_empty() {
            col = col.push(item(style::secondary("No services listed")));
        }
        for (i, s) in self.services.iter().enumerate() {
            let state = self.states.get(i).map(String::as_str).unwrap_or("");
            let sub = match self.busy.iter().find(|(u, _)| u == &s.unit) {
                Some((_, b)) => b.to_string(),
                None => match state {
                    "active" => "Running".to_string(),
                    "failed" => "Failed".to_string(),
                    "" => String::new(),
                    _ => "Stopped".to_string(),
                },
            };
            let label = Column::new().push(text::body(s.label.clone())).push(style::secondary_small(sub)).width(Length::Fill);
            col = col.push(item(
                row![label, toggler(running(state)).on_toggle(move |on| Message::Set(i, on))].align_y(Alignment::Center).into(),
            ));
        }
        if let Some(e) = &self.error {
            col = col.push(item(style::secondary(e.clone())));
        }
        col = col
            .push(style::separator())
            .push(button::custom(text::body("Edit Services…")).class(Button::AppletMenu).padding([4, 14]).width(Length::Fill).on_press(Message::Edit));
        style::popover(col.spacing(2).padding([10, 0]), 300.0)
    }

    fn subscription(&self) -> Subscription<Message> {
        let status = Subscription::run_with((self.popup.is_some(), self.services.clone()), |(open, services)| {
            let period = Duration::from_secs(if *open { 2 } else { 30 });
            let services = services.clone();
            let list = services.clone();
            watch::changes(period, move |_: Option<Vec<String>>| gather(services.clone())).map(move |st| (list.clone(), st))
        });
        Subscription::batch([
            status.map(|(services, states)| Message::States(services, states)),
            watch::open_requests(APP_ID).map(|_| Message::OpenRequest),
        ])
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::CloseRequested(id))
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
