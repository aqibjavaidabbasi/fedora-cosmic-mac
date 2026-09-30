//! macOS-style Bluetooth item for the COSMIC panel.
//!
//! Panel: Bluetooth glyph (dimmed when off).
//! Popover (macOS Tahoe layout): Bluetooth switch, Devices (paired; blue when
//! connected, battery %), Nearby Devices (expandable, scans), Bluetooth Settings…
//! The same menu is drawn inside the Control Center (shared/menus.rs). Uses bluetoothctl.

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

const APP_ID: &str = "com.aqib.CosmicMacBluetooth";

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<Bluetooth>(())
}

#[derive(Default)]
struct Bluetooth {
    core: app::Core,
    popup: Option<window::Id>,
    state: bt::State,
    show_nearby: bool,
    busy: Option<String>, // MAC being connected/disconnected
}

#[derive(Debug, Clone)]
enum Message {
    TogglePopup,
    CloseRequested(window::Id),
    Tick,
    CheckOpen,
    Refreshed(bt::State),
    Enable(bool),
    Device(bt::Device),
    ToggleNearby,
    OpenSettings,
}

fn run(program: &str, args: &[&str]) {
    if let Ok(mut child) = std::process::Command::new(program).args(args).spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

impl Bluetooth {
    fn panel_icon(&self) -> &'static str {
        if !self.state.powered {
            "bluetooth-disabled-symbolic"
        } else if self.state.devices.iter().any(|d| d.connected) {
            "bluetooth-active-symbolic"
        } else {
            "bluetooth-symbolic"
        }
    }

    fn refresh(&self) -> app::Task<Message> {
        let nearby = self.show_nearby;
        cosmic::task::future(async move { cosmic::Action::App(Message::Refreshed(bt::gather(nearby).await)) })
    }

    fn close_popup(&mut self) -> app::Task<Message> {
        match self.popup.take() {
            Some(p) => surface::surface_task(surface::action::destroy_popup(p)),
            None => Task::none(),
        }
    }
}

impl cosmic::Application for Bluetooth {
    type Message = Message;
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    const APP_ID: &'static str = APP_ID;

    fn init(core: app::Core, _flags: ()) -> (Self, app::Task<Message>) {
        let app = Self { core, ..Default::default() };
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
                self.show_nearby = false;
                let new_id = window::Id::unique();
                let open = surface::surface_task(surface::action::app_popup(
                    |_| Default::default(),
                    move |app: &mut Bluetooth| {
                        app.popup.replace(new_id);
                        app.core.applet.get_popup_settings(app.core.main_window_id().unwrap(), new_id, Some((1, 1)), None, None)
                    },
                    None,
                )).chain(style::blur(new_id));
                return Task::batch([open, self.refresh()]);
            }
            Message::CloseRequested(id) => {
                if Some(id) == self.popup {
                    self.popup = None;
                }
            }
            Message::Tick => return self.refresh(),
            Message::CheckOpen => {
                if style::open_requested(APP_ID) {
                    return self.update(Message::TogglePopup);
                }
            }
            Message::Refreshed(state) => {
                if let Some(mac) = &self.busy {
                    let before = self.state.devices.iter().find(|d| &d.mac == mac).map(|d| d.connected);
                    let after = state.devices.iter().find(|d| &d.mac == mac).map(|d| d.connected);
                    if before != after {
                        self.busy = None;
                    }
                }
                self.state = state;
            }
            Message::Enable(on) => {
                self.state.powered = on;
                bt::set_powered(on);
            }
            Message::Device(d) => {
                bt::toggle(&d);
                self.busy = Some(d.mac);
            }
            Message::ToggleNearby => {
                self.show_nearby = !self.show_nearby;
                if self.show_nearby {
                    bt::scan();
                    return self.refresh();
                }
            }
            Message::OpenSettings => {
                run("cosmic-settings", &["bluetooth"]);
                return self.close_popup();
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        self.core.applet.icon_button(self.panel_icon()).padding(style::bar_padding(&self.core.applet)).on_press_down(Message::TogglePopup).into()
    }

    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let menu = menus::bluetooth(
            &self.state,
            self.show_nearby,
            self.busy.as_deref(),
            menus::BtMsgs {
                toggle: Message::Enable,
                device: Message::Device,
                nearby: Message::ToggleNearby,
                settings: Message::OpenSettings,
                back: None,
            },
        );
        style::popover(Column::new().push(menu).padding([10, 0]), 300.0)
    }

    fn subscription(&self) -> Subscription<Message> {
        let period = if self.popup.is_some() { 2000 } else { 10000 };
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
