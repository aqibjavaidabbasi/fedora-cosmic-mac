//! macOS Tahoe-style Control Center for the COSMIC panel.
//!
//! Grid, matching macOS:
//!   [Wi-Fi      ] [ Now Playing  ]
//!   [Bluetooth  ] [              ]
//!   [Airplane   ] (Overview) (Displays)
//!   (Dark)(Shot)  [Focus         ]
//!   [Display ─────────────────○  ]
//!   [Sound   ─────○           (⎋)]
//!   [Record Selection] [Capture  ]
//!   [Record Screen   ] [Record   ]
//!   (Lock)(Log Out)(Restart)(Shut Down)        [Edit Controls]
//!
//! It drives the same services COSMIC's own applets use, through their command-line
//! tools: nmcli, rfkill + bluetoothctl, wpctl, playerctl (MPRIS), cosmic-settings-daemon
//! over D-Bus (brightness), the cosmic-notifications config (Focus = Do Not Disturb),
//! the COSMIC theme mode, cosmic-screenshot, gpu-screen-recorder (if installed),
//! loginctl and cosmic-osd.

mod style;

use std::{process::Command, time::Duration};

use cosmic::{
    Element, Task, app,
    iced::{Alignment, Background, Border, Color, Length, Subscription, core::widget::text::Wrapping, time, window},
    iced::widget::slider as iced_slider,
    theme::{self, Button},
    widget::{Column, Row, button, container, icon, image, row, slider, space, text},
};

const APP_ID: &str = "com.aqib.CosmicMacControlCenter";
const WIDTH: f32 = 376.0;
const GAP: f32 = 10.0;
const PAD: f32 = 12.0;
const TILE_H: f32 = 56.0;
const COL_W: f32 = (WIDTH - 2.0 * PAD - GAP) / 2.0;
const CIRCLE: f32 = TILE_H;

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<ControlCenter>(())
}

#[derive(Clone, Debug, Default)]
struct Media {
    playing: bool,
    title: String,
    artist: String,
    art: Option<String>, // local file path
}

#[derive(Clone, Debug, Default)]
struct Status {
    wifi_on: bool,
    wifi_name: Option<String>,
    bluetooth_on: bool,
    airplane: bool,
    dnd: bool,
    dark: bool,
    volume: u32,
    muted: bool,
    brightness: Option<(i32, i32)>, // (current, max)
    media: Option<Media>,
    recording: bool,
    recorder: bool, // gpu-screen-recorder available
}

#[derive(Default)]
struct ControlCenter {
    core: app::Core,
    popup: Option<window::Id>,
    status: Status,
}

#[derive(Debug, Clone)]
enum Message {
    TogglePopup,
    CloseRequested(window::Id),
    CheckOpen,
    Refresh,
    Refreshed(Status),
    Wifi,
    Bluetooth,
    Airplane,
    Focus,
    DarkMode,
    Volume(u32),
    Brightness(i32),
    Media(&'static str),
    Screenshot,
    Record { region: bool },
    Open(&'static str),
    Overview,
    Lock,
    LogOut,
    Restart,
    ShutDown,
}

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

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

fn dnd_path() -> String {
    format!("{}/.config/cosmic/com.system76.CosmicNotifications/v1/do_not_disturb", home())
}

fn dark_path() -> String {
    format!("{}/.config/cosmic/com.system76.CosmicTheme.Mode/v1/is_dark", home())
}

fn recorder_pidfile() -> String {
    format!("{}/cosmic-mac-recording.pid", std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into()))
}

fn which(bin: &str) -> bool {
    std::env::var("PATH").unwrap_or_default().split(':').any(|d| std::path::Path::new(d).join(bin).is_file())
}

async fn gather() -> Status {
    let mut s = Status::default();

    s.wifi_on = output("nmcli", &["-t", "radio", "wifi"]).await.trim() == "enabled";
    s.wifi_name = output("nmcli", &["-t", "-f", "active,ssid", "dev", "wifi"])
        .await
        .lines()
        .find_map(|l| l.strip_prefix("yes:").map(|n| n.replace("\\:", ":")))
        .filter(|n| !n.is_empty());

    let bt = output("bluetoothctl", &["show"]).await;
    s.bluetooth_on = bt.lines().any(|l| l.trim() == "Powered: yes");

    let rf = output("rfkill", &["-n", "-o", "SOFT"]).await;
    s.airplane = !rf.trim().is_empty() && rf.lines().all(|l| l.trim() == "blocked");

    s.dnd = tokio::fs::read_to_string(dnd_path()).await.map(|v| v.trim() == "true").unwrap_or(false);
    s.dark = tokio::fs::read_to_string(dark_path()).await.map(|v| v.trim() == "true").unwrap_or(true);

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

    let meta = output(
        "playerctl",
        &["metadata", "--format", "{{status}}\u{1f}{{title}}\u{1f}{{artist}}\u{1f}{{mpris:artUrl}}"],
    )
    .await;
    let f: Vec<&str> = meta.trim_end_matches('\n').split('\u{1f}').collect();
    if f.len() == 4 && !f[1].is_empty() {
        s.media = Some(Media {
            playing: f[0] == "Playing",
            title: f[1].to_string(),
            artist: f[2].to_string(),
            art: f[3].strip_prefix("file://").map(str::to_string),
        });
    }

    s.recorder = which("gpu-screen-recorder");
    s.recording = std::fs::read_to_string(recorder_pidfile())
        .ok()
        .and_then(|p| p.trim().parse::<u32>().ok())
        .is_some_and(|pid| std::path::Path::new(&format!("/proc/{pid}")).exists());
    s
}

fn refresh_task() -> app::Task<Message> {
    cosmic::task::future(async { cosmic::Action::App(Message::Refreshed(gather().await)) })
}

// ---------- styles ----------

fn tile_fill(t: &cosmic::Theme) -> Color {
    if t.cosmic().is_dark { Color::from_rgba(1.0, 1.0, 1.0, 0.10) } else { Color::from_rgba(0.0, 0.0, 0.0, 0.06) }
}

fn tile_style(radius: f32, active: bool, hover: f32, t: &cosmic::Theme) -> cosmic::widget::button::Style {
    let mut fill = if active { Color::WHITE } else { tile_fill(t) };
    if !active {
        fill.a += hover;
    }
    let fg: Color = if active { Color::BLACK } else { t.cosmic().background(t.transparent).on.into() };
    cosmic::widget::button::Style {
        background: Some(Background::Color(fill)),
        border_radius: radius.into(),
        icon_color: Some(fg),
        text_color: Some(fg),
        ..cosmic::widget::button::Style::new()
    }
}

/// A glass tile/circle as a button: `radius` 999 makes pills and circles.
fn tile_button(radius: f32, active: bool) -> Button {
    Button::Custom {
        active: Box::new(move |_, t| tile_style(radius, active, 0.0, t)),
        disabled: Box::new(move |t| tile_style(radius, active, 0.0, t)),
        hovered: Box::new(move |_, t| tile_style(radius, active, 0.06, t)),
        pressed: Box::new(move |_, t| tile_style(radius, active, 0.12, t)),
    }
}

/// Round glyph chip inside a pill: Mac blue with a white glyph when on.
fn chip(on: bool) -> theme::Container<'static> {
    theme::Container::custom(move |t| {
        let (fill, fg) = if on {
            (style::MAC_BLUE, Color::WHITE)
        } else if t.cosmic().is_dark {
            (Color::from_rgba(1.0, 1.0, 1.0, 0.16), Color::WHITE)
        } else {
            (Color::from_rgba(0.0, 0.0, 0.0, 0.10), Color::BLACK)
        };
        cosmic::iced::widget::container::Style {
            background: Some(Background::Color(fill)),
            border: Border { radius: 999.0.into(), ..Default::default() },
            icon_color: Some(fg),
            text_color: Some(fg),
            ..Default::default()
        }
    })
}

fn card_class() -> theme::Container<'static> {
    theme::Container::custom(|t| cosmic::iced::widget::container::Style {
        background: Some(Background::Color(tile_fill(t))),
        border: Border { radius: 22.0.into(), ..Default::default() },
        ..Default::default()
    })
}

fn slider_style(t: &cosmic::Theme) -> iced_slider::Style {
    let track = if t.cosmic().is_dark { Color::from_rgba(1.0, 1.0, 1.0, 0.18) } else { Color::from_rgba(0.0, 0.0, 0.0, 0.10) };
    iced_slider::Style {
        rail: iced_slider::Rail {
            backgrounds: (Background::Color(Color::WHITE), Background::Color(track)),
            width: 22.0,
            border: Border { radius: 11.0.into(), ..Default::default() },
        },
        handle: iced_slider::Handle {
            shape: iced_slider::HandleShape::Circle { radius: 11.0 },
            background: Background::Color(Color::WHITE),
            border_width: 0.5,
            border_color: Color::from_rgba(0.0, 0.0, 0.0, 0.15),
        },
        breakpoint: iced_slider::Breakpoint { color: Color::TRANSPARENT },
    }
}

/// Thick macOS slider: white filled rail, faint track, white round knob.
fn mac_slider() -> theme::iced::Slider {
    theme::iced::Slider::Custom {
        active: std::rc::Rc::new(slider_style),
        hovered: std::rc::Rc::new(slider_style),
        dragging: std::rc::Rc::new(slider_style),
    }
}

// ---------- building blocks ----------

fn glyph<'a>(name: &'static str, size: u16) -> Element<'a, Message> {
    icon::from_name(name).size(size).symbolic(true).into()
}

fn chip_icon<'a>(name: &'static str, on: bool) -> Element<'a, Message> {
    container(glyph(name, 16))
        .width(Length::Fixed(34.0))
        .height(Length::Fixed(34.0))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .class(chip(on))
        .into()
}

/// Pill tile: (chip) Title / subtitle.
fn pill<'a>(icon_name: &'static str, title: &'static str, subtitle: Option<String>, on: bool, msg: Message) -> Element<'a, Message> {
    let mut labels = Column::new().push(text::heading(title).size(13).wrapping(Wrapping::None));
    if let Some(sub) = subtitle {
        labels = labels.push(style::secondary_small(sub));
    }
    button::custom(
        container(row![chip_icon(icon_name, on), labels].spacing(8).align_y(Alignment::Center))
            .height(Length::Fill)
            .align_y(Alignment::Center),
    )
    .class(tile_button(999.0, false))
    .padding([0, 10])
        .width(Length::Fixed(COL_W))
        .height(Length::Fixed(TILE_H))
        .on_press(msg)
        .into()
}

/// Round button with a single glyph.
fn circle<'a>(icon_name: &'static str, active: bool, msg: Message) -> Element<'a, Message> {
    button::custom(
        container(glyph(icon_name, 20)).width(Length::Fill).height(Length::Fill).align_x(Alignment::Center).align_y(Alignment::Center),
    )
    .class(tile_button(999.0, active))
    .width(Length::Fixed(CIRCLE))
    .height(Length::Fixed(CIRCLE))
    .padding(0)
    .on_press(msg)
    .into()
}

/// Two round buttons sharing one grid column, spread like macOS.
fn circle_pair<'a>(a: Element<'a, Message>, b: Element<'a, Message>) -> Element<'a, Message> {
    row![a, space::horizontal(), b].width(Length::Fixed(COL_W)).into()
}

impl ControlCenter {
    fn close_popup(&mut self) -> app::Task<Message> {
        match self.popup.take() {
            Some(p) => cosmic::surface::surface_task(cosmic::surface::action::destroy_popup(p)),
            None => Task::none(),
        }
    }

    fn now_playing(&self) -> Element<'_, Message> {
        let h = 2.0 * TILE_H + GAP;
        let content: Element<'_, Message> = match &self.status.media {
            Some(m) => {
                let art: Element<'_, Message> = match &m.art {
                    Some(path) => image(path.clone()).width(Length::Fixed(40.0)).height(Length::Fixed(40.0)).border_radius([8.0; 4]).into(),
                    None => chip_icon("audio-x-generic-symbolic", false),
                };
                let ctl = |name: &'static str, action: &'static str| {
                    button::custom(glyph(name, 18)).class(Button::Icon).padding(3).on_press(Message::Media(action))
                };
                Column::new()
                    .push(art)
                    .push(space::vertical().height(Length::Fill))
                    .push(text::heading(m.title.clone()).size(13).wrapping(Wrapping::None))
                    .push(style::secondary_small(m.artist.clone()))
                    .push(
                        row![
                            ctl("media-skip-backward-symbolic", "previous"),
                            space::horizontal(),
                            ctl(if m.playing { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" }, "play-pause"),
                            space::horizontal(),
                            ctl("media-skip-forward-symbolic", "next"),
                        ]
                        .align_y(Alignment::Center),
                    )
                    .spacing(2)
                    .into()
            }
            None => Column::new()
                .push(chip_icon("audio-x-generic-symbolic", false))
                .push(space::vertical().height(Length::Fill))
                .push(text::heading("Not Playing").size(13))
                .push(style::secondary_small("Music"))
                .spacing(2)
                .into(),
        };
        container(content).padding(12).width(Length::Fixed(COL_W)).height(Length::Fixed(h)).clip(true).class(card_class()).into()
    }

    fn slider_card<'a>(&self, title: &'static str, low: &'static str, bar: Element<'a, Message>, high: Element<'a, Message>) -> Element<'a, Message> {
        container(
            Column::new()
                .push(text::heading(title).size(13))
                .push(row![glyph(low, 14), bar, high].spacing(8).align_y(Alignment::Center))
                .spacing(8),
        )
        .padding([10, 14])
        .width(Length::Fill)
        .class(card_class())
        .into()
    }
}

impl cosmic::Application for ControlCenter {
    type Message = Message;
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    const APP_ID: &'static str = APP_ID;

    fn init(core: app::Core, _flags: ()) -> (Self, app::Task<Message>) {
        (Self { core, ..Default::default() }, refresh_task())
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
                let open = cosmic::surface::surface_task(cosmic::surface::action::app_popup(
                    |_| Default::default(),
                    |app: &mut ControlCenter| {
                        let new_id = window::Id::unique();
                        app.popup.replace(new_id);
                        app.core.applet.get_popup_settings(app.core.main_window_id().unwrap(), new_id, Some((1, 1)), None, None)
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
            Message::CheckOpen => {
                if style::open_requested(APP_ID) {
                    return self.update(Message::TogglePopup);
                }
            }
            Message::Refresh => return refresh_task(),
            Message::Refreshed(status) => self.status = status,
            Message::Wifi => {
                let s = &mut self.status;
                s.wifi_on = !s.wifi_on;
                run("nmcli", &["radio", "wifi", if s.wifi_on { "on" } else { "off" }]);
            }
            Message::Bluetooth => {
                let s = &mut self.status;
                s.bluetooth_on = !s.bluetooth_on;
                if s.bluetooth_on {
                    let _ = Command::new("rfkill").args(["unblock", "bluetooth"]).status();
                    run("bluetoothctl", &["power", "on"]);
                } else {
                    run("bluetoothctl", &["power", "off"]);
                }
            }
            Message::Airplane => {
                let s = &mut self.status;
                s.airplane = !s.airplane;
                run("rfkill", &[if s.airplane { "block" } else { "unblock" }, "all"]);
            }
            Message::Focus => {
                let s = &mut self.status;
                s.dnd = !s.dnd;
                let _ = std::fs::write(dnd_path(), if s.dnd { "true" } else { "false" });
            }
            Message::DarkMode => {
                let s = &mut self.status;
                s.dark = !s.dark;
                let _ = std::fs::write(dark_path(), if s.dark { "true" } else { "false" });
            }
            Message::Volume(v) => {
                let s = &mut self.status;
                s.volume = v;
                run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{v}%")]);
                if s.muted && v > 0 {
                    s.muted = false;
                    run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "0"]);
                }
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
            Message::Media(action) => {
                if action == "play-pause" {
                    if let Some(m) = self.status.media.as_mut() {
                        m.playing = !m.playing;
                    }
                }
                run("playerctl", &[action]);
            }
            Message::Screenshot => {
                run("cosmic-screenshot", &[]);
                return self.close_popup();
            }
            Message::Record { region } => {
                if self.status.recording {
                    if let Ok(pid) = std::fs::read_to_string(recorder_pidfile()) {
                        run("kill", &["-INT", pid.trim()]);
                    }
                    self.status.recording = false;
                } else if self.status.recorder {
                    let file = format!("{}/Videos/Recording {}.mp4", home(), stamp());
                    let target = if region { "region" } else { "portal" };
                    if let Ok(mut child) = Command::new("gpu-screen-recorder").args(["-w", target, "-f", "60", "-o", &file]).spawn() {
                        let _ = std::fs::write(recorder_pidfile(), child.id().to_string());
                        self.status.recording = true;
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                    }
                    return self.close_popup();
                } else {
                    // No recorder yet: open the app store so one can be installed.
                    run("cosmic-store", &[]);
                    return self.close_popup();
                }
            }
            Message::Open(page) => {
                run("cosmic-settings", &[page]);
                return self.close_popup();
            }
            Message::Overview => {
                run("cosmic-workspaces", &[]);
                return self.close_popup();
            }
            Message::Lock => run("loginctl", &["lock-session"]),
            Message::LogOut => run("cosmic-osd", &["log-out"]),
            Message::Restart => run("cosmic-osd", &["restart"]),
            Message::ShutDown => run("cosmic-osd", &["shutdown"]),
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        self.core.applet.icon_button("com.aqib.control-center-symbolic").on_press_down(Message::TogglePopup).into()
    }

    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let s = &self.status;
        let on_off = |on: bool| Some(if on { "On" } else { "Off" }.to_string());

        // Connectivity pills + Now Playing
        let wifi_sub = if s.wifi_on { s.wifi_name.clone().unwrap_or_else(|| "Not Connected".into()) } else { "Off".into() };
        let conn = Column::new()
            .push(pill("network-wireless-signal-excellent-symbolic", "Wi-Fi", Some(wifi_sub), s.wifi_on, Message::Wifi))
            .push(pill("bluetooth-active-symbolic", "Bluetooth", on_off(s.bluetooth_on), s.bluetooth_on, Message::Bluetooth))
            .spacing(GAP);
        let block1 = row![conn, self.now_playing()].spacing(GAP);

        let block2 = row![
            pill("airplane-mode-symbolic", "Airplane Mode", on_off(s.airplane), s.airplane, Message::Airplane),
            circle_pair(
                circle("view-app-grid-symbolic", false, Message::Overview),
                circle("video-display-symbolic", false, Message::Open("displays")),
            ),
        ]
        .spacing(GAP);

        let block3 = row![
            circle_pair(
                circle("display-brightness-symbolic", !s.dark, Message::DarkMode),
                circle("camera-photo-symbolic", false, Message::Screenshot),
            ),
            pill("weather-clear-night-symbolic", "Focus", s.dnd.then(|| "Do Not Disturb".to_string()), s.dnd, Message::Focus),
        ]
        .spacing(GAP);

        let mut grid = Column::new().push(block1).push(block2).push(block3).spacing(GAP);

        if let Some((cur, max)) = s.brightness {
            grid = grid.push(self.slider_card(
                "Display",
                "display-brightness-low-symbolic",
                slider((max / 20)..=max, cur, Message::Brightness).step((max / 100).max(1)).height(22).class(mac_slider()).into(),
                glyph("display-brightness-high-symbolic", 16),
            ));
        }
        let vol_icon = if s.muted || s.volume == 0 { "audio-volume-muted-symbolic" } else { "audio-volume-low-symbolic" };
        grid = grid.push(self.slider_card(
            "Sound",
            vol_icon,
            slider(0..=100u32, s.volume.min(100), Message::Volume).height(22).class(mac_slider()).into(),
            row![
                glyph("audio-volume-high-symbolic", 16),
                button::custom(glyph("audio-speakers-symbolic", 16)).class(tile_button(999.0, false)).padding(6).on_press(Message::Open("sound")),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
        ));

        let rec_sub = if !s.recorder {
            Some("Not Installed".to_string())
        } else {
            on_off(s.recording)
        };
        grid = grid
            .push(
                row![
                    pill("selection-mode-symbolic", "Record Selection", rec_sub.clone(), s.recording, Message::Record { region: true }),
                    pill("applets-screenshooter-symbolic", "Capture", None, false, Message::Screenshot),
                ]
                .spacing(GAP),
            )
            .push(
                row![
                    pill("media-record-symbolic", "Record Screen", rec_sub, s.recording, Message::Record { region: false }),
                    pill("camera-video-symbolic", "Record", on_off(s.recording), s.recording, Message::Record { region: false }),
                ]
                .spacing(GAP),
            );

        // Power row (macOS keeps these in the Apple menu; COSMIC has none, so they live here).
        let power = |name: &'static str, msg: Message| button::custom(glyph(name, 16)).class(tile_button(999.0, false)).padding(9).on_press(msg);
        let footer = Row::new()
            .push(power("system-lock-screen-symbolic", Message::Lock))
            .push(power("system-log-out-symbolic", Message::LogOut))
            .push(power("system-reboot-symbolic", Message::Restart))
            .push(power("system-shutdown-symbolic", Message::ShutDown))
            .push(space::horizontal())
            .push(button::custom(text::body("Edit Controls")).class(tile_button(999.0, false)).padding([6, 14]).on_press(Message::Open("desktop")))
            .spacing(8)
            .align_y(Alignment::Center);

        let content = Column::new().push(grid).push(footer).spacing(14).padding(PAD);
        style::popover(content, WIDTH)
    }

    fn subscription(&self) -> Subscription<Message> {
        let open = time::every(Duration::from_millis(500)).map(|_| Message::CheckOpen);
        if self.popup.is_some() {
            Subscription::batch([open, time::every(Duration::from_secs(2)).map(|_| Message::Refresh)])
        } else {
            open
        }
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::CloseRequested(id))
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

/// "2026-09-29 at 17.30.12", like macOS recording names.
fn stamp() -> String {
    Command::new("date")
        .arg("+%Y-%m-%d at %H.%M.%S")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}
