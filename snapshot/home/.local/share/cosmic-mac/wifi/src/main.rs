//! macOS-style Wi-Fi item for the COSMIC panel.
//!
//! Panel: Wi-Fi waves showing signal strength.
//! Popover (macOS Tahoe layout): Wi-Fi switch, Weak Security…, Known Networks,
//! Other Networks (expandable), Wi-Fi Settings…  The same menu is drawn inside the
//! Control Center (shared/menus.rs). Talks to NetworkManager through nmcli.

#[path = "../../shared/style.rs"]
mod style;
#[path = "../../shared/net.rs"]
mod net;
#[path = "../../shared/bt.rs"]
mod bt;
#[path = "../../shared/menus.rs"]
mod menus;

use std::time::Duration;

use cosmic::{
    Element, Task, app,
    iced::{Subscription, time, window},
    surface,
    widget::Column,
};

const APP_ID: &str = "com.aqib.CosmicMacWifi";

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<Wifi>(())
}

#[derive(Default)]
struct Wifi {
    core: app::Core,
    popup: Option<window::Id>,
    state: net::State,
    show_other: bool,
    connecting: Option<String>,
}

#[derive(Debug, Clone)]
enum Message {
    TogglePopup,
    CloseRequested(window::Id),
    Tick,
    CheckOpen,
    Refreshed(net::State),
    Enable(bool),
    Connect(net::Network),
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

impl Wifi {
    fn panel_icon(&self) -> &'static str {
        if !self.state.enabled {
            return "network-wireless-disabled-symbolic";
        }
        match self.state.current() {
            Some(n) => net::signal_icon(n.signal),
            None => "network-wireless-offline-symbolic",
        }
    }

    fn refresh(&self, rescan: bool) -> app::Task<Message> {
        // Saved networks only change when the user saves one: look them up when the menu opens.
        let known = (!rescan).then(|| self.state.known_ssids());
        cosmic::task::future(async move { cosmic::Action::App(Message::Refreshed(net::gather(rescan, known).await)) })
    }

    fn close_popup(&mut self) -> app::Task<Message> {
        match self.popup.take() {
            Some(p) => surface::surface_task(surface::action::destroy_popup(p)),
            None => Task::none(),
        }
    }
}

impl cosmic::Application for Wifi {
    type Message = Message;
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    const APP_ID: &'static str = APP_ID;

    fn init(core: app::Core, _flags: ()) -> (Self, app::Task<Message>) {
        let app = Self { core, ..Default::default() };
        let t = app.refresh(true);
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
                if self.connecting.as_ref().is_some_and(|c| state.current().is_some_and(|n| &n.ssid == c)) {
                    self.connecting = None;
                }
                self.state = state;
            }
            Message::Enable(on) => {
                self.state.enabled = on;
                if !on {
                    self.state.networks.clear();
                }
                net::set_enabled(on);
            }
            Message::Connect(n) => {
                if n.in_use {
                    return Task::none();
                }
                if net::connect(&n) {
                    self.connecting = Some(n.ssid);
                } else {
                    // New secured network: COSMIC Settings asks for the password.
                    run("cosmic-settings", &["wireless"]);
                    return self.close_popup();
                }
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
        let menu = menus::wifi(
            &self.state,
            self.show_other,
            self.connecting.as_deref(),
            menus::WifiMsgs {
                toggle: Message::Enable,
                connect: Message::Connect,
                other: Message::ToggleOther,
                settings: Message::OpenSettings,
                back: None,
            },
        );
        style::popover(Column::new().push(menu).padding([10, 0]), 300.0)
    }

    fn subscription(&self) -> Subscription<Message> {
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
