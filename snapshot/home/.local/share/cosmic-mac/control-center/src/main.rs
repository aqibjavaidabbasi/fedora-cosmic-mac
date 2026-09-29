//! macOS-style Control Center for the COSMIC panel.
//!
//! One panel icon; its popup holds Wi-Fi, Bluetooth and Do Not Disturb toggles,
//! display brightness and volume sliders, battery status, shortcuts into COSMIC
//! Settings, and Lock / Log Out / Restart / Shut Down.
//!
//! It drives the same system services COSMIC's own applets use, through their
//! command-line tools: nmcli (NetworkManager), rfkill + bluetoothctl (BlueZ),
//! wpctl (PipeWire), cosmic-settings-daemon over D-Bus (brightness), the
//! cosmic-notifications config (Do Not Disturb), loginctl and cosmic-osd.

use std::{process::Command, time::Duration};

use cosmic::{
    Element, Task, app,
    applet::{menu_button, padded_control},
    iced::{Alignment, Length, Subscription, time, window},
    theme,
    widget::{Column, divider, row, slider, text, toggler},
};

const APP_ID: &str = "com.aqib.CosmicMacControlCenter";

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<ControlCenter>(())
}

#[derive(Clone, Debug, Default)]
struct Status {
    wifi_on: bool,
    wifi_name: Option<String>,
    bluetooth_on: bool,
    dnd: bool,
    volume: u32,
    muted: bool,
    brightness: Option<(i32, i32)>, // (current, max)
    battery: Option<(u32, String)>, // (percent, status)
}

#[derive(Default)]
struct ControlCenter {
    core: cosmic::app::Core,
    popup: Option<window::Id>,
    status: Status,
}

#[derive(Debug, Clone)]
enum Message {
    TogglePopup,
    CloseRequested(window::Id),
    Refresh,
    Refreshed(Status),
    Wifi(bool),
    Bluetooth(bool),
    DoNotDisturb(bool),
    Volume(u32),
    Mute(bool),
    Brightness(i32),
    Open(&'static str),
    Lock,
    LogOut,
    Restart,
    ShutDown,
}

/// Run a command in the background, reaping it on a thread.
fn run(program: &str, args: &[&str]) {
    if let Ok(mut child) = Command::new(program).args(args).spawn() {
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

fn dnd_path() -> String {
    format!(
        "{}/.config/cosmic/com.system76.CosmicNotifications/v1/do_not_disturb",
        std::env::var("HOME").unwrap_or_default()
    )
}

async fn gather() -> Status {
    let mut s = Status::default();

    s.wifi_on = output("nmcli", &["-t", "radio", "wifi"]).await.trim() == "enabled";
    s.wifi_name = output("nmcli", &["-t", "-f", "active,ssid", "dev", "wifi"])
        .await
        .lines()
        .find_map(|l| l.strip_prefix("yes:").map(str::to_string))
        .filter(|n| !n.is_empty());

    let bt = output("bluetoothctl", &["show"]).await;
    s.bluetooth_on = bt.lines().any(|l| l.trim() == "Powered: yes");

    s.dnd = tokio::fs::read_to_string(dnd_path())
        .await
        .map(|v| v.trim() == "true")
        .unwrap_or(false);

    // "Volume: 0.40 [MUTED]"
    let vol = output("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]).await;
    s.muted = vol.contains("[MUTED]");
    s.volume = vol
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse::<f64>().ok())
        .map(|v| (v * 100.0).round().clamp(0.0, 150.0) as u32)
        .unwrap_or(0);

    let prop = |name: &'static str| async move {
        output(
            "busctl",
            &[
                "--user",
                "get-property",
                "com.system76.CosmicSettingsDaemon",
                "/com/system76/CosmicSettingsDaemon",
                "com.system76.CosmicSettingsDaemon",
                name,
            ],
        )
        .await
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse::<i32>().ok())
    };
    if let (Some(cur), Some(max)) = (prop("DisplayBrightness").await, prop("MaxDisplayBrightness").await) {
        if max > 0 && cur >= 0 {
            s.brightness = Some((cur, max));
        }
    }

    if let Ok(mut dir) = tokio::fs::read_dir("/sys/class/power_supply").await {
        while let Ok(Some(entry)) = dir.next_entry().await {
            let p = entry.path();
            let kind = tokio::fs::read_to_string(p.join("type")).await.unwrap_or_default();
            if kind.trim() != "Battery" {
                continue;
            }
            let cap = tokio::fs::read_to_string(p.join("capacity")).await.unwrap_or_default();
            let st = tokio::fs::read_to_string(p.join("status")).await.unwrap_or_default();
            if let Ok(cap) = cap.trim().parse() {
                s.battery = Some((cap, st.trim().to_string()));
                break;
            }
        }
    }
    s
}

fn refresh_task() -> app::Task<Message> {
    cosmic::task::future(async { cosmic::Action::App(Message::Refreshed(gather().await)) })
}

impl cosmic::Application for ControlCenter {
    type Message = Message;
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    const APP_ID: &'static str = APP_ID;

    fn init(core: cosmic::app::Core, _flags: ()) -> (Self, app::Task<Message>) {
        (Self { core, ..Default::default() }, refresh_task())
    }

    fn core(&self) -> &cosmic::app::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::app::Core {
        &mut self.core
    }

    fn update(&mut self, message: Message) -> app::Task<Message> {
        match message {
            Message::TogglePopup => {
                if let Some(p) = self.popup.take() {
                    return cosmic::surface::surface_task(cosmic::surface::action::destroy_popup(p));
                }
                let open = cosmic::surface::surface_task(cosmic::surface::action::app_popup(
                    |_| Default::default(),
                    |app: &mut ControlCenter| {
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
                return Task::batch([open, refresh_task()]);
            }
            Message::CloseRequested(id) => {
                if Some(id) == self.popup {
                    self.popup = None;
                }
            }
            Message::Refresh => return refresh_task(),
            Message::Refreshed(status) => self.status = status,
            Message::Wifi(on) => {
                self.status.wifi_on = on;
                run("nmcli", &["radio", "wifi", if on { "on" } else { "off" }]);
            }
            Message::Bluetooth(on) => {
                self.status.bluetooth_on = on;
                if on {
                    // unblock the radio first, then power the adapter
                    let _ = Command::new("rfkill").args(["unblock", "bluetooth"]).status();
                    run("bluetoothctl", &["power", "on"]);
                } else {
                    run("bluetoothctl", &["power", "off"]);
                }
            }
            Message::DoNotDisturb(on) => {
                self.status.dnd = on;
                let _ = std::fs::write(dnd_path(), if on { "true" } else { "false" });
            }
            Message::Volume(v) => {
                self.status.volume = v;
                run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{v}%")]);
                if self.status.muted && v > 0 {
                    self.status.muted = false;
                    run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "0"]);
                }
            }
            Message::Mute(sound_on) => {
                self.status.muted = !sound_on;
                run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", if sound_on { "0" } else { "1" }]);
            }
            Message::Brightness(v) => {
                if let Some((_, max)) = self.status.brightness {
                    self.status.brightness = Some((v, max));
                    run(
                        "busctl",
                        &[
                            "--user",
                            "set-property",
                            "com.system76.CosmicSettingsDaemon",
                            "/com/system76/CosmicSettingsDaemon",
                            "com.system76.CosmicSettingsDaemon",
                            "DisplayBrightness",
                            "i",
                            &v.to_string(),
                        ],
                    );
                }
            }
            Message::Open(page) => {
                run("cosmic-settings", &[page]);
                if let Some(p) = self.popup.take() {
                    return cosmic::surface::surface_task(cosmic::surface::action::destroy_popup(p));
                }
            }
            Message::Lock => run("loginctl", &["lock-session"]),
            Message::LogOut => run("cosmic-osd", &["log-out"]),
            Message::Restart => run("cosmic-osd", &["restart"]),
            Message::ShutDown => run("cosmic-osd", &["shutdown"]),
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        self.core
            .applet
            .icon_button("preferences-system-symbolic")
            .on_press_down(Message::TogglePopup)
            .into()
    }

    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let cosmic::cosmic_theme::Spacing { space_xxs, space_s, .. } = theme::active().cosmic().spacing;
        let s = &self.status;

        let wifi_label = match (&s.wifi_name, s.wifi_on) {
            (Some(name), true) => format!("Wi-Fi  ·  {name}"),
            (_, true) => "Wi-Fi  ·  not connected".to_string(),
            _ => "Wi-Fi".to_string(),
        };
        let wifi = padded_control(
            toggler(s.wifi_on).on_toggle(Message::Wifi).label(wifi_label).text_size(14).width(Length::Fill),
        );
        let bluetooth = padded_control(
            toggler(s.bluetooth_on)
                .on_toggle(Message::Bluetooth)
                .label("Bluetooth".to_string())
                .text_size(14)
                .width(Length::Fill),
        );
        let dnd = padded_control(
            toggler(s.dnd)
                .on_toggle(Message::DoNotDisturb)
                .label("Do Not Disturb".to_string())
                .text_size(14)
                .width(Length::Fill),
        );

        let sep = || padded_control(divider::horizontal::default()).padding([space_xxs, space_s]);

        let mut content = Column::new().push(wifi).push(bluetooth).push(dnd).push(sep());

        if let Some((cur, max)) = s.brightness {
            content = content.push(padded_control(
                Column::new()
                    .spacing(4)
                    .push(text::body("Display"))
                    .push(slider(max / 20..=max, cur, Message::Brightness).step(max / 100)),
            ));
        }
        content = content.push(padded_control(
            Column::new()
                .spacing(4)
                .push(
                    row![
                        text::body(format!("Sound  ·  {}%", s.volume)).width(Length::Fill),
                        toggler(!s.muted).on_toggle(Message::Mute).label("On".to_string()).text_size(12),
                    ]
                    .align_y(Alignment::Center),
                )
                .push(slider(0..=100u32, s.volume.min(100), Message::Volume)),
        ));

        if let Some((pct, st)) = &s.battery {
            let state = match st.as_str() {
                "Charging" => "  ·  charging",
                "Full" => "  ·  full",
                "Not charging" => "  ·  plugged in",
                _ => "",
            };
            content = content.push(padded_control(text::body(format!("Battery  {pct}%{state}"))));
        }

        content = content
            .push(sep())
            .push(menu_button(text::body("Wi-Fi & Network Settings…")).on_press(Message::Open("wireless")))
            .push(menu_button(text::body("Bluetooth Settings…")).on_press(Message::Open("bluetooth")))
            .push(menu_button(text::body("Sound Settings…")).on_press(Message::Open("sound")))
            .push(menu_button(text::body("Displays Settings…")).on_press(Message::Open("displays")))
            .push(sep())
            .push(menu_button(text::body("Lock Screen")).on_press(Message::Lock))
            .push(menu_button(text::body("Log Out…")).on_press(Message::LogOut))
            .push(menu_button(text::body("Restart…")).on_press(Message::Restart))
            .push(menu_button(text::body("Shut Down…")).on_press(Message::ShutDown))
            .padding([8, 0]);

        self.core.applet.popup_container(content).into()
    }

    fn subscription(&self) -> Subscription<Message> {
        // Keep the popup live while it is open.
        if self.popup.is_some() {
            time::every(Duration::from_secs(2)).map(|_| Message::Refresh)
        } else {
            Subscription::none()
        }
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::CloseRequested(id))
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
