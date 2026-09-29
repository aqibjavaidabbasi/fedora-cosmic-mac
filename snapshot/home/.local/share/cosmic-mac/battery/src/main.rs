//! macOS-style battery item for the COSMIC panel.
//!
//! Panel: "54%" + battery glyph (yellow while Low Power Mode is on, bolt while charging).
//! Popover: Battery header, power source, Energy Mode card with a Low Power Mode
//! toggle, apps using significant energy, and "Battery Settings…".
//!
//! State comes from UPower and power-profiles (tuned-ppd on Fedora) over D-Bus;
//! see `power.rs`. No polling except the /proc sampler while the popover is open.

mod energy;
mod power;
#[path = "../../shared/style.rs"]
mod style;

use std::time::Duration;

use cosmic::{
    Element, Task, app,
    iced::{Alignment, Length, Subscription, time, window},
    surface,
    theme::Button,
    widget::{Column, button, container, icon, row, text},
};

use power::{Charge, Event, Snapshot};

const APP_ID: &str = "com.aqib.CosmicMacBattery";

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<Battery>(())
}

#[derive(Default)]
struct Battery {
    core: app::Core,
    popup: Option<window::Id>,
    snap: Option<Snapshot>,
    conn: Option<zbus::Connection>,
    error: Option<String>,
    sampler: energy::Sampler,
    heavy_apps: Vec<String>,
}

#[derive(Debug, Clone)]
enum Message {
    TogglePopup,
    CloseRequested(window::Id),
    Power(Event),
    LowPower(bool),
    LowPowerDone(Result<(), String>),
    SampleEnergy,
    OpenSettings,
    CheckOpenRequest,
}

/// Run a command in the background, reaping it on a thread.
fn run(program: &str, args: &[&str]) {
    if let Ok(mut child) = std::process::Command::new(program).args(args).spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

fn format_remaining(d: Duration) -> String {
    let mins = d.as_secs() / 60;
    format!("{}:{:02}", mins / 60, mins % 60)
}

impl Battery {
    fn panel_icon(&self) -> &'static str {
        let Some(s) = self.snap.as_ref().filter(|s| s.present) else {
            return "battery-missing-symbolic";
        };
        if s.charge == Charge::Full || (s.percent >= 99.5 && !s.on_battery) {
            return "battery-level-100-charged-symbolic";
        }
        const LEVELS: [&str; 11] = [
            "battery-level-0-symbolic", "battery-level-10-symbolic", "battery-level-20-symbolic",
            "battery-level-30-symbolic", "battery-level-40-symbolic", "battery-level-50-symbolic",
            "battery-level-60-symbolic", "battery-level-70-symbolic", "battery-level-80-symbolic",
            "battery-level-90-symbolic", "battery-level-100-symbolic",
        ];
        const CHARGING: [&str; 11] = [
            "battery-level-0-charging-symbolic", "battery-level-10-charging-symbolic",
            "battery-level-20-charging-symbolic", "battery-level-30-charging-symbolic",
            "battery-level-40-charging-symbolic", "battery-level-50-charging-symbolic",
            "battery-level-60-charging-symbolic", "battery-level-70-charging-symbolic",
            "battery-level-80-charging-symbolic", "battery-level-90-charging-symbolic",
            "battery-level-100-charged-symbolic",
        ];
        let i = ((s.percent / 10.0).round() as usize).min(10);
        if s.charge == Charge::Charging { CHARGING[i] } else { LEVELS[i] }
    }

    fn close_popup(&mut self) -> app::Task<Message> {
        match self.popup.take() {
            Some(p) => surface::surface_task(surface::action::destroy_popup(p)),
            None => Task::none(),
        }
    }
}

impl cosmic::Application for Battery {
    type Message = Message;
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    const APP_ID: &'static str = APP_ID;

    fn init(core: app::Core, _flags: ()) -> (Self, app::Task<Message>) {
        (Self { core, ..Default::default() }, Task::none())
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
                self.sampler.sample(); // prime, so the next tick has a baseline
                return surface::surface_task(surface::action::app_popup(
                    |_| Default::default(),
                    |app: &mut Battery| {
                        let new_id = window::Id::unique();
                        app.popup.replace(new_id);
                        app.core.applet.get_popup_settings(
                            app.core.main_window_id().unwrap(),
                            new_id,
                            Some((1, 1)),
                            None,
                            None,
                        )
                    },
                    None,
                ));
            }
            Message::CloseRequested(id) => {
                if Some(id) == self.popup {
                    self.popup = None;
                }
            }
            Message::Power(Event::Connected(conn)) => self.conn = Some(conn),
            Message::Power(Event::Update(snap)) => {
                self.snap = Some(snap);
                self.error = None;
            }
            Message::Power(Event::Error(e)) => self.error = Some(e),
            Message::LowPower(on) => {
                // optimistic; the D-Bus property signal confirms or reverts it
                if let Some(s) = self.snap.as_mut() {
                    s.low_power = Some(on);
                }
                if let Some(conn) = self.conn.clone() {
                    return cosmic::task::future(async move {
                        cosmic::Action::App(Message::LowPowerDone(power::set_low_power(conn, on).await))
                    });
                }
            }
            Message::LowPowerDone(Ok(())) => {}
            Message::LowPowerDone(Err(e)) => self.error = Some(e),
            Message::SampleEnergy => self.heavy_apps = self.sampler.sample(),
            Message::CheckOpenRequest => {
                if style::open_requested(APP_ID) {
                    return self.update(Message::TogglePopup);
                }
            }
            Message::OpenSettings => {
                run("cosmic-settings", &["power"]);
                return self.close_popup();
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let (icon_size, _) = self.core.applet.suggested_size(true);
        let (pad_major, pad_minor) = self.core.applet.suggested_padding(true);
        let low_power = self.snap.as_ref().and_then(|s| s.low_power).unwrap_or(false);

        let mut content = row![].spacing(pad_minor).align_y(Alignment::Center);
        if let Some(s) = self.snap.as_ref().filter(|s| s.present) {
            content = content.push(self.core.applet.text(format!("{:.0}%", s.percent)));
        }
        content = content.push(icon::from_name(self.panel_icon()).size(icon_size).symbolic(true));

        let btn = button::custom(container(content).class(style::low_power_tint(low_power)))
            .on_press_down(Message::TogglePopup)
            .class(Button::AppletIcon)
            .padding([pad_minor, pad_major]);

        self.core.applet.autosize_window(btn).into()
    }

    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        // Layout follows the macOS Tahoe battery menu:
        //   Battery / Power Source: … / ─ / Energy Mode / (●) Low Power / ─ /
        //   No Apps Using Significant Energy / ─ / Battery Settings…
        let item = |content: Element<'static, Message>| container(content).padding([4, 14]).width(Length::Fill);

        let Some(s) = &self.snap else {
            let msg = self.error.clone().unwrap_or_else(|| "Reading battery…".to_string());
            return style::popover(Column::new().push(item(style::secondary(msg))).padding([10, 0]), 300.0);
        };

        let mut status = Column::new().spacing(2).push(style::secondary(if s.on_battery {
            "Power Source: Battery"
        } else {
            "Power Source: Power Adapter"
        }));
        let remaining = match (s.charge, s.remaining) {
            (Charge::Charging, Some(d)) => Some(format!("{} until full", format_remaining(d))),
            (Charge::NotCharging, _) => Some("Not Charging".to_string()),
            _ => None,
        };
        if let Some(r) = remaining {
            status = status.push(style::secondary(r));
        }

        // Energy Mode: a round badge that is blue while Low Power is on; the whole row toggles it.
        let low_power_on = s.low_power.unwrap_or(false);
        let lp_badge = container(icon::from_name("battery-level-30-symbolic").size(16).symbolic(true))
            .width(Length::Fixed(28.0))
            .height(Length::Fixed(28.0))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .class(style::badge(low_power_on));
        let mut lp_row = button::custom(
            row![lp_badge, text::body("Low Power")].spacing(10).align_y(Alignment::Center),
        )
        .class(Button::AppletMenu)
        .padding([4, 14])
        .width(Length::Fill);
        if s.low_power.is_some() {
            lp_row = lp_row.on_press(Message::LowPower(!low_power_on));
        }

        let mut apps = Column::new().spacing(4);
        if self.heavy_apps.is_empty() {
            apps = apps.push(style::secondary("No Apps Using Significant Energy"));
        } else {
            apps = apps.push(style::section("Using Significant Energy"));
            for name in &self.heavy_apps {
                apps = apps.push(text::body(name.clone()));
            }
        }

        let mut content = Column::new()
            .push(item(text::heading("Battery").into()))
            .push(item(status.into()))
            .push(style::separator())
            .push(item(style::section("Energy Mode")))
            .push(lp_row)
            .push(style::separator())
            .push(item(apps.into()));
        if let Some(e) = &self.error {
            content = content.push(item(style::secondary(format!("⚠ {e}"))));
        }
        content = content
            .push(style::separator())
            .push(
                button::custom(text::body("Battery Settings…"))
                    .class(Button::AppletMenu)
                    .padding([4, 14])
                    .width(Length::Fill)
                    .on_press(Message::OpenSettings),
            )
            .padding([10, 0])
            .spacing(2);

        style::popover(content, 300.0)
    }

    fn subscription(&self) -> Subscription<Message> {
        let power = power::subscription().map(Message::Power);
        let open = time::every(Duration::from_millis(500)).map(|_| Message::CheckOpenRequest);
        if self.popup.is_some() {
            Subscription::batch([power, open, time::every(Duration::from_secs(3)).map(|_| Message::SampleEnergy)])
        } else {
            Subscription::batch([power, open])
        }
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::CloseRequested(id))
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
