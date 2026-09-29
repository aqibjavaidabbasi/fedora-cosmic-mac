//! macOS-style Wi-Fi item for the COSMIC panel.
//!
//! Panel: Wi-Fi waves showing signal strength.
//! Popover (macOS Tahoe layout):
//!   Wi-Fi                               [toggle]
//!   Weak Security…                           ⚠      (only when the current network is weak)
//!   ─
//!   Known Networks
//!   (●) Network                              🔒
//!   ─
//!   Other Networks                           ›
//!   ─
//!   Wi-Fi Settings…
//!
//! Talks to NetworkManager through nmcli.

mod style;

use std::time::Duration;

use cosmic::{
    Element, Task, app,
    iced::{Alignment, Length, Subscription, time, window},
    surface,
    theme::Button,
    widget::{Column, button, container, icon, row, space, text, toggler},
};

const APP_ID: &str = "com.aqib.CosmicMacWifi";

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<Wifi>(())
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Network {
    ssid: String,
    signal: u8,
    security: String,
    in_use: bool,
    known: bool,
}

impl Network {
    fn secured(&self) -> bool {
        !self.security.trim().is_empty()
    }
    /// Open, WEP, or still allowing WPA1 (TKIP): what macOS flags as "Weak Security".
    fn weak(&self) -> bool {
        let s = self.security.to_uppercase();
        s.trim().is_empty() || s.contains("WEP") || s.contains("WPA1")
    }
}

#[derive(Clone, Debug, Default)]
struct State {
    enabled: bool,
    networks: Vec<Network>, // strongest first, one entry per SSID
}

impl State {
    fn current(&self) -> Option<&Network> {
        self.networks.iter().find(|n| n.in_use)
    }
}

#[derive(Default)]
struct Wifi {
    core: app::Core,
    popup: Option<window::Id>,
    state: State,
    show_other: bool,
    connecting: Option<String>,
}

#[derive(Debug, Clone)]
enum Message {
    TogglePopup,
    CloseRequested(window::Id),
    Tick,
    CheckOpen,
    Refreshed(State),
    Enable(bool),
    Connect(Network),
    ToggleOther,
    OpenSettings,
}

fn run(program: &str, args: &[&str]) {
    if let Ok(mut child) = std::process::Command::new(program).args(args).spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

async fn output(program: &str, args: &[&str]) -> String {
    tokio::process::Command::new(program)
        .args(args)
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

/// Split one `nmcli -t` line on unescaped ':'.
fn fields(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    out.last_mut().unwrap().push(n);
                }
            }
            ':' => out.push(String::new()),
            _ => out.last_mut().unwrap().push(c),
        }
    }
    out
}

async fn known_ssids() -> Vec<String> {
    let mut ssids = Vec::new();
    for line in output("nmcli", &["-t", "-f", "NAME,TYPE", "connection", "show"]).await.lines() {
        let f = fields(line);
        if f.get(1).map(String::as_str) == Some("802-11-wireless") {
            let ssid = output("nmcli", &["-g", "802-11-wireless.ssid", "connection", "show", &f[0]]).await;
            let ssid = ssid.trim();
            ssids.push(if ssid.is_empty() { f[0].clone() } else { ssid.to_string() });
        }
    }
    ssids
}

async fn gather(rescan: bool, known: Option<Vec<String>>) -> State {
    let enabled = output("nmcli", &["-t", "radio", "wifi"]).await.trim() == "enabled";
    if !enabled {
        return State { enabled, networks: Vec::new() };
    }
    let known = match known {
        Some(k) => k,
        None => known_ssids().await,
    };
    let list = output(
        "nmcli",
        &["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "dev", "wifi", "list", "--rescan", if rescan { "yes" } else { "no" }],
    )
    .await;
    let mut networks: Vec<Network> = Vec::new();
    for line in list.lines() {
        let f = fields(line);
        if f.len() < 4 || f[1].trim().is_empty() {
            continue;
        }
        let n = Network {
            in_use: f[0] == "*",
            ssid: f[1].clone(),
            signal: f[2].parse().unwrap_or(0),
            security: f[3].clone(),
            known: known.contains(&f[1]),
        };
        match networks.iter_mut().find(|m| m.ssid == n.ssid) {
            Some(m) => {
                m.in_use |= n.in_use;
                if n.signal > m.signal {
                    m.signal = n.signal;
                }
            }
            None => networks.push(n),
        }
    }
    networks.sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)));
    State { enabled, networks }
}

fn signal_icon(signal: u8) -> &'static str {
    match signal {
        70.. => "network-wireless-signal-excellent-symbolic",
        50..70 => "network-wireless-signal-good-symbolic",
        30..50 => "network-wireless-signal-ok-symbolic",
        _ => "network-wireless-signal-weak-symbolic",
    }
}

impl Wifi {
    fn panel_icon(&self) -> &'static str {
        if !self.state.enabled {
            return "network-wireless-disabled-symbolic";
        }
        match self.state.current() {
            Some(n) => signal_icon(n.signal),
            None => "network-wireless-offline-symbolic",
        }
    }

    fn refresh(&self, rescan: bool) -> app::Task<Message> {
        // Known SSIDs only change when the user saves a network; look them up only when the popover opens.
        let known = (!rescan).then(|| {
            self.state.networks.iter().filter(|n| n.known).map(|n| n.ssid.clone()).collect::<Vec<_>>()
        });
        cosmic::task::future(async move { cosmic::Action::App(Message::Refreshed(gather(rescan, known).await)) })
    }

    fn close_popup(&mut self) -> app::Task<Message> {
        match self.popup.take() {
            Some(p) => surface::surface_task(surface::action::destroy_popup(p)),
            None => Task::none(),
        }
    }

    fn network_row(&self, n: &Network) -> Element<'_, Message> {
        let active = n.in_use;
        let badge = container(icon::from_name(signal_icon(n.signal)).size(16).symbolic(true))
            .width(Length::Fixed(28.0))
            .height(Length::Fixed(28.0))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .class(style::badge(active));
        let label = if self.connecting.as_deref() == Some(n.ssid.as_str()) {
            format!("{}  (connecting…)", n.ssid)
        } else {
            n.ssid.clone()
        };
        let mut r = row![badge, text::body(label).width(Length::Fill)].spacing(10).align_y(Alignment::Center);
        if n.secured() {
            r = r.push(icon::from_name("system-lock-screen-symbolic").size(14).symbolic(true));
        }
        button::custom(r)
            .class(Button::AppletMenu)
            .padding([3, 14])
            .width(Length::Fill)
            .on_press(Message::Connect(n.clone()))
            .into()
    }
}

impl cosmic::Application for Wifi {
    type Message = Message;
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    const APP_ID: &'static str = APP_ID;

    fn init(core: app::Core, _flags: ()) -> (Self, app::Task<Message>) {
        let app = Self { core, ..Default::default() };
        let t = app.refresh(false);
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
                self.show_other = false;
                let open = surface::surface_task(surface::action::app_popup(
                    |_| Default::default(),
                    |app: &mut Wifi| {
                        let new_id = window::Id::unique();
                        app.popup.replace(new_id);
                        app.core.applet.get_popup_settings(app.core.main_window_id().unwrap(), new_id, Some((1, 1)), None, None)
                    },
                    None,
                ));
                return Task::batch([open, self.refresh(true)]);
            }
            Message::CloseRequested(id) => {
                if Some(id) == self.popup {
                    self.popup = None;
                }
            }
            Message::Tick => return self.refresh(false),
            Message::CheckOpen => {
                if style::open_requested(APP_ID) {
                    return self.update(Message::TogglePopup);
                }
            }
            Message::Refreshed(state) => {
                if let Some(c) = &self.connecting {
                    if state.current().is_some_and(|n| &n.ssid == c) {
                        self.connecting = None;
                    }
                }
                self.state = state;
            }
            Message::Enable(on) => {
                self.state.enabled = on;
                if !on {
                    self.state.networks.clear();
                }
                run("nmcli", &["radio", "wifi", if on { "on" } else { "off" }]);
            }
            Message::Connect(n) => {
                if n.in_use {
                    return Task::none();
                }
                if n.known {
                    run("nmcli", &["connection", "up", "id", &n.ssid]);
                } else if !n.secured() {
                    run("nmcli", &["device", "wifi", "connect", &n.ssid]);
                } else {
                    // New secured network: COSMIC Settings asks for the password.
                    run("cosmic-settings", &["wireless"]);
                    return self.close_popup();
                }
                self.connecting = Some(n.ssid);
            }
            Message::ToggleOther => self.show_other = !self.show_other,
            Message::OpenSettings => {
                run("cosmic-settings", &["wireless"]);
                return self.close_popup();
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        self.core.applet.icon_button(self.panel_icon()).on_press_down(Message::TogglePopup).into()
    }

    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let item = |content: Element<'static, Message>| container(content).padding([4, 14]).width(Length::Fill);
        let s = &self.state;

        let header = item(
            row![text::heading("Wi-Fi").size(15).width(Length::Fill), toggler(s.enabled).on_toggle(Message::Enable)]
                .align_y(Alignment::Center)
                .into(),
        );
        let mut content = Column::new().push(header);

        if s.enabled {
            if s.current().is_some_and(Network::weak) {
                content = content.push(
                    button::custom(
                        row![
                            text::body("Weak Security…").width(Length::Fill),
                            container(icon::from_name("dialog-warning-symbolic").size(16).symbolic(true))
                                .class(style::low_power_tint(true)),
                        ]
                        .align_y(Alignment::Center),
                    )
                    .class(Button::AppletMenu)
                    .padding([4, 14])
                    .width(Length::Fill)
                    .on_press(Message::OpenSettings),
                );
            }

            let known: Vec<&Network> = s.networks.iter().filter(|n| n.known || n.in_use).collect();
            if !known.is_empty() {
                content = content.push(style::separator()).push(item(style::section("Known Networks")));
                for n in known {
                    content = content.push(self.network_row(n));
                }
            }

            content = content.push(style::separator()).push(
                button::custom(
                    row![
                        text::heading("Other Networks").width(Length::Fill),
                        icon::from_name(if self.show_other { "go-down-symbolic" } else { "go-next-symbolic" })
                            .size(14)
                            .symbolic(true),
                    ]
                    .align_y(Alignment::Center),
                )
                .class(Button::AppletMenu)
                .padding([4, 14])
                .width(Length::Fill)
                .on_press(Message::ToggleOther),
            );
            if self.show_other {
                let others: Vec<&Network> = s.networks.iter().filter(|n| !n.known && !n.in_use).collect();
                if others.is_empty() {
                    content = content.push(item(style::secondary("No other networks")));
                }
                for n in others {
                    content = content.push(self.network_row(n));
                }
            }
        }

        content = content
            .push(style::separator())
            .push(
                button::custom(text::body("Wi-Fi Settings…"))
                    .class(Button::AppletMenu)
                    .padding([4, 14])
                    .width(Length::Fill)
                    .on_press(Message::OpenSettings),
            )
            .push(space::vertical().height(Length::Fixed(2.0)))
            .padding([10, 0])
            .spacing(2);

        style::popover(content, 300.0)
    }

    fn subscription(&self) -> Subscription<Message> {
        // Open requests are checked often; network state every tick while open, every 5 s otherwise.
        let period = if self.popup.is_some() { 2000 } else { 5000 };
        Subscription::batch([
            time::every(Duration::from_millis(period)).map(|_| Message::Tick),
            time::every(Duration::from_millis(500)).map(|_| Message::CheckOpen),
        ])
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::CloseRequested(id))
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
