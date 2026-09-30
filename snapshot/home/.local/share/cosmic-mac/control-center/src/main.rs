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
//! the COSMIC theme mode, cosmic-screenshot, GPU Screen Recorder (Flatpak or native),
//! loginctl and cosmic-osd.

#[path = "../../shared/style.rs"]
mod style;
#[path = "../../shared/net.rs"]
mod net;
#[path = "../../shared/bt.rs"]
mod bt;
#[path = "../../shared/menus.rs"]
mod menus;

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
    touch: Option<Touch>, // None when there is no touchscreen
    dnd: bool,
    dark: bool,
    volume: u32,
    muted: bool,
    brightness: Option<(i32, i32)>, // (current, max)
    media: Option<Media>,
    recording: bool,
    recorder: bool, // gpu-screen-recorder available
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Page {
    #[default]
    Main,
    Wifi,
    Bluetooth,
    Customize,
}

#[derive(Default)]
struct ControlCenter {
    core: app::Core,
    popup: Option<window::Id>,
    status: Status,
    page: Page,
    wifi: net::State,
    show_other: bool,
    connecting: Option<String>,
    bt: bt::State,
    show_nearby: bool,
    busy: Option<String>,
    bar_icon: Option<String>,
    custom_icon: String,
    meters: Vec<String>,
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
    ShowPage(Page),
    WifiState(net::State),
    WifiEnable(bool),
    WifiConnect(net::Network),
    WifiOther,
    BtState(bt::State),
    BtEnable(bool),
    BtDevice(bt::Device),
    BtNearby,
    Airplane,
    Touch,
    Focus,
    DarkMode,
    Volume(u32),
    Brightness(i32),
    Media(&'static str),
    Screenshot,
    Record { choose: bool },
    Open(&'static str),
    Overview,
    SetBarIcon(Option<String>),
    CustomIcon(String),
    ApplyCustomIcon,
    ToggleMeter(&'static str),
    EditWidgets,
    Lock,
    LogOut,
    Restart,
    ShutDown,
}

/// The touchscreen: its sysfs "inhibited" switches, whether it is on, and whether we may flip it.
#[derive(Clone, Debug, Default)]
struct Touch {
    switches: Vec<std::path::PathBuf>,
    on: bool,
    writable: bool,
}

/// Touchscreens: devices udev tags ID_INPUT_TOUCHSCREEN, plus Surface IPTS touch devices.
fn find_touch() -> Option<Touch> {
    let mut t = Touch::default();
    for e in std::fs::read_dir("/sys/class/input").ok()?.flatten() {
        let dir = e.path();
        let name = std::fs::read_to_string(dir.join("name")).unwrap_or_default();
        let name = name.trim();
        let is_touch = name == "Intel Touch Host Controller" || {
            // udev's database, keyed by the event node's major:minor
            std::fs::read_dir(&dir).into_iter().flatten().flatten().any(|c| {
                c.file_name().to_string_lossy().starts_with("event")
                    && std::fs::read_to_string(c.path().join("dev"))
                        .ok()
                        .and_then(|d| std::fs::read_to_string(format!("/run/udev/data/c{}", d.trim())).ok())
                        .is_some_and(|db| db.contains("E:ID_INPUT_TOUCHSCREEN=1"))
            })
        };
        let switch = dir.join("inhibited");
        if is_touch && switch.exists() {
            t.switches.push(switch);
        }
    }
    if t.switches.is_empty() {
        return None;
    }
    t.on = t.switches.iter().any(|p| std::fs::read_to_string(p).is_ok_and(|v| v.trim() == "0"));
    t.writable = t.switches.iter().all(|p| std::fs::OpenOptions::new().write(true).open(p).is_ok());
    Some(t)
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

const GSR_FLATPAK: &str = "com.dec05eba.gpu_screen_recorder";

/// How to start GPU Screen Recorder: the native binary, or its Flatpak.
fn recorder_command() -> Option<Vec<String>> {
    if which("gpu-screen-recorder") {
        return Some(vec!["gpu-screen-recorder".into()]);
    }
    let installed = [
        format!("{}/.local/share/flatpak/app/{GSR_FLATPAK}", home()),
        format!("/var/lib/flatpak/app/{GSR_FLATPAK}"),
    ]
    .iter()
    .any(|p| std::path::Path::new(p).exists());
    installed.then(|| vec!["flatpak".into(), "run".into(), "--command=gpu-screen-recorder".into(), GSR_FLATPAK.into()])
}

/// The recorder process (its name is cut to 15 characters by the kernel).
fn recording_now() -> bool {
    Command::new("pgrep").args(["-x", "gpu-screen-reco"]).output().is_ok_and(|o| o.status.success())
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
    s.touch = find_touch();

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

    s.recorder = recorder_command().is_some();
    s.recording = recording_now();
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

// ---------- "Edit Controls": customization written to config files ----------

const BAR_ICONS: [(&str, &str); 6] = [
    ("com.system76.CosmicAppLibrary", "Default"),
    ("distributor-logo-fedora", "Fedora"),
    ("distributor-logo-pop-os", "COSMIC"),
    ("distributor-logo", "Linux"),
    ("view-app-grid-symbolic", "Grid"),
    ("start-here-symbolic", "Start"),
];
const METERS: [(&str, &str); 4] = [("cpu", "CPU"), ("memory", "Memory"), ("network", "Network"), ("disk", "Disk")];

fn panel_button_conf() -> String {
    format!("{}/.config/cosmic/com.system76.CosmicPanelButton/v1/configs", home())
}

fn stats_conf() -> String {
    format!("{}/.config/cosmic-mac/stats.conf", home())
}

/// The left-most top-bar button's custom icon (App Library button).
fn read_bar_icon() -> Option<String> {
    let text = std::fs::read_to_string(panel_button_conf()).ok()?;
    let line = text.lines().find(|l| l.contains("\"*/com.system76.CosmicAppLibrary\""))?;
    let rest = line.split("icon: Some(\"").nth(1)?;
    Some(rest.split('"').next()?.to_string())
}

fn write_bar_icon(icon: Option<&str>) {
    let path = panel_button_conf();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| "{\n}".to_string());
    let entry = match icon {
        Some(i) => format!("    \"*/com.system76.CosmicAppLibrary\": (force_presentation: Some(Icon), icon: Some({i:?})),"),
        None => "    \"*/com.system76.CosmicAppLibrary\": (force_presentation: Some(Icon)),".to_string(),
    };
    let mut lines: Vec<String> = text.lines().filter(|l| !l.contains("\"*/com.system76.CosmicAppLibrary\"")).map(str::to_string).collect();
    let at = lines.iter().position(|l| l.trim() == "{").map_or(0, |i| i + 1);
    lines.insert(at, entry);
    let _ = std::fs::create_dir_all(std::path::Path::new(&path).parent().unwrap());
    let _ = std::fs::write(&path, lines.join("\n"));
}

fn read_meters() -> Vec<String> {
    std::fs::read_to_string(stats_conf())
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.trim().strip_prefix("SHOW=").map(|v| v.split(',').map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect()))
        .unwrap_or_else(|| vec!["cpu".into(), "memory".into()])
}

fn write_meters(list: &[String]) {
    let _ = std::fs::create_dir_all(format!("{}/.config/cosmic-mac", home()));
    let _ = std::fs::write(
        stats_conf(),
        format!("# Meters shown in the top bar, left to right: cpu, memory, network, disk\nSHOW={}\n", list.join(",")),
    );
}

/// Wi-Fi/Bluetooth pill, like macOS: the round chip toggles, the rest opens the menu.
fn split_pill<'a>(icon_name: &'static str, title: &'static str, subtitle: String, on: bool, toggle: Message, open: Message) -> Element<'a, Message> {
    let chip = button::custom(chip_icon(icon_name, on)).class(Button::Text).padding(0).on_press(toggle);
    let labels = button::custom(
        container(
            Column::new()
                .push(text::heading(title).size(13).wrapping(Wrapping::None))
                .push(style::secondary_small(subtitle)),
        )
        .height(Length::Fill)
        .width(Length::Fill)
        .align_y(Alignment::Center),
    )
    .class(Button::Text)
    .padding(0)
    .width(Length::Fill)
    .height(Length::Fill)
    .on_press(open.clone());
    button::custom(container(row![chip, labels].spacing(8).align_y(Alignment::Center)).height(Length::Fill).align_y(Alignment::Center))
        .class(tile_button(999.0, false))
        .padding([0, 10])
        .width(Length::Fixed(COL_W))
        .height(Length::Fixed(TILE_H))
        .on_press(open)
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
    fn customize_view(&self) -> Element<'_, Message> {
        let selected = self.bar_icon.clone().unwrap_or_else(|| BAR_ICONS[0].0.to_string());
        let mut icons = Row::new().spacing(8);
        for (name, label) in BAR_ICONS {
            let symbolic = name.ends_with("-symbolic");
            let b = button::custom(
                Column::new()
                    .push(icon::from_name(name).size(26).symbolic(symbolic))
                    .push(text::caption(label).size(10))
                    .align_x(Alignment::Center)
                    .spacing(2),
            )
            .class(tile_button(12.0, selected == name))
            .padding([6, 4])
            .width(Length::Fill)
            .on_press(Message::SetBarIcon(Some(name.to_string())));
            icons = icons.push(b);
        }
        let custom = row![
            cosmic::widget::text_input("Icon name or /path/to/image.svg", self.custom_icon.clone())
                .on_input(Message::CustomIcon)
                .on_submit(|_| Message::ApplyCustomIcon)
                .width(Length::Fill),
            button::custom(text::body("Use")).class(tile_button(999.0, false)).padding([5, 12]).on_press(Message::ApplyCustomIcon),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let mut meters = Column::new().spacing(4);
        for (key, label) in METERS {
            let on = self.meters.iter().any(|m| m == key);
            meters = meters.push(
                row![text::body(label).width(Length::Fill), cosmic::widget::toggler(on).on_toggle(move |_| Message::ToggleMeter(key))]
                    .align_y(Alignment::Center),
            );
        }

        let card = |c: Element<'static, Message>| container(c).padding([10, 14]).width(Length::Fill).class(card_class());
        let content = Column::new()
            .push(
                row![
                    button::custom(icon::from_name("go-previous-symbolic").size(16).symbolic(true))
                        .class(Button::Icon)
                        .padding(2)
                        .on_press(Message::ShowPage(Page::Main)),
                    text::heading("Edit Controls").size(15),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .push(style::section("Menu Bar Icon"))
            .push(card(Column::new().push(icons).push(custom).spacing(10).into()))
            .push(style::section("System Monitor in Menu Bar"))
            .push(card(meters.into()))
            .push(style::section("Widgets"))
            .push(
                row![
                    button::custom(text::body("Edit Widgets…")).class(tile_button(999.0, false)).padding([6, 14]).on_press(Message::EditWidgets),
                    button::custom(text::body("COSMIC Settings…")).class(tile_button(999.0, false)).padding([6, 14]).on_press(Message::Open("desktop")),
                ]
                .spacing(8),
            )
            .spacing(10)
            .padding(PAD);
        style::popover(content, WIDTH)
    }

    fn refresh_page(&self, rescan: bool) -> app::Task<Message> {
        match self.page {
            Page::Main | Page::Customize => Task::none(),
            Page::Wifi => {
                let known = (!rescan).then(|| self.wifi.known_ssids());
                cosmic::task::future(async move { cosmic::Action::App(Message::WifiState(net::gather(rescan, known).await)) })
            }
            Page::Bluetooth => {
                let nearby = self.show_nearby;
                cosmic::task::future(async move { cosmic::Action::App(Message::BtState(bt::gather(nearby).await)) })
            }
        }
    }

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
                self.page = Page::Main;
                let new_id = window::Id::unique();
                let open = cosmic::surface::surface_task(cosmic::surface::action::app_popup(
                    |_| Default::default(),
                    move |app: &mut ControlCenter| {
                        app.popup.replace(new_id);
                        app.core.applet.get_popup_settings(app.core.main_window_id().unwrap(), new_id, Some((1, 1)), None, None)
                    },
                    None,
                )).chain(style::blur(new_id));
                return Task::batch([open, refresh_task()]);
            }
            Message::CloseRequested(id) => {
                if Some(id) == self.popup {
                    self.popup = None;
                }
            }
            Message::CheckOpen => {
                // The request file may name a page: `echo wifi > …/open/<app id>` opens Wi-Fi.
                let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default();
                let page = std::fs::read_to_string(format!("{dir}/cosmic-mac/open/{APP_ID}")).unwrap_or_default();
                if style::open_requested(APP_ID) {
                    let target = match page.trim() {
                        "wifi" => Page::Wifi,
                        "bluetooth" => Page::Bluetooth,
                        "customize" => Page::Customize,
                        _ => Page::Main,
                    };
                    let opening = self.popup.is_none();
                    let toggle = self.update(Message::TogglePopup);
                    if opening && target != Page::Main {
                        return Task::batch([toggle, self.update(Message::ShowPage(target))]);
                    }
                    return toggle;
                }
            }
            Message::Refresh => {
                return match self.page {
                    Page::Main => refresh_task(),
                    _ => self.refresh_page(false),
                };
            }
            Message::Refreshed(status) => self.status = status,
            Message::ShowPage(page) => {
                if page == Page::Customize {
                    self.bar_icon = read_bar_icon();
                    self.custom_icon = self.bar_icon.clone().filter(|i| !BAR_ICONS.iter().any(|(k, _)| k == i)).unwrap_or_default();
                    self.meters = read_meters();
                }
                self.page = page;
                self.show_other = false;
                self.show_nearby = false;
                return self.refresh_page(true);
            }
            Message::WifiState(state) => {
                if self.connecting.as_ref().is_some_and(|c| state.current().is_some_and(|n| &n.ssid == c)) {
                    self.connecting = None;
                }
                self.status.wifi_on = state.enabled;
                self.wifi = state;
            }
            Message::WifiEnable(on) => {
                self.wifi.enabled = on;
                self.status.wifi_on = on;
                if !on {
                    self.wifi.networks.clear();
                }
                net::set_enabled(on);
            }
            Message::WifiConnect(n) => {
                if !n.in_use {
                    if net::connect(&n) {
                        self.connecting = Some(n.ssid);
                    } else {
                        run("cosmic-settings", &["wireless"]);
                        return self.close_popup();
                    }
                }
            }
            Message::WifiOther => self.show_other = !self.show_other,
            Message::BtState(state) => {
                if let Some(mac) = &self.busy {
                    let before = self.bt.devices.iter().find(|d| &d.mac == mac).map(|d| d.connected);
                    let after = state.devices.iter().find(|d| &d.mac == mac).map(|d| d.connected);
                    if before != after {
                        self.busy = None;
                    }
                }
                self.status.bluetooth_on = state.powered;
                self.bt = state;
            }
            Message::BtEnable(on) => {
                self.bt.powered = on;
                self.status.bluetooth_on = on;
                bt::set_powered(on);
            }
            Message::BtDevice(d) => {
                bt::toggle(&d);
                self.busy = Some(d.mac);
            }
            Message::BtNearby => {
                self.show_nearby = !self.show_nearby;
                if self.show_nearby {
                    bt::scan();
                    return self.refresh_page(false);
                }
            }
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
            Message::Touch => {
                if let Some(t) = self.status.touch.as_mut() {
                    if t.writable {
                        t.on = !t.on;
                        for p in &t.switches {
                            let _ = std::fs::write(p, if t.on { "0" } else { "1" });
                        }
                    } else {
                        // one-time setup: lets you switch the touchscreen without a password
                        let script = format!("{}/.local/share/cosmic-mac/touch-setup.sh", home());
                        run("pkexec", &["/bin/sh", &script]);
                    }
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
            Message::Record { choose } => {
                if self.status.recording {
                    // Stop: GPU Screen Recorder finishes the file on SIGINT.
                    run("pkill", &["-INT", "-x", "gpu-screen-reco"]);
                    self.status.recording = false;
                } else if let Some(cmd) = recorder_command() {
                    // Both go through COSMIC's screen-share dialog (no root helper needed):
                    // "Record Selection" always asks what to record; "Record Screen" reuses
                    // the last choice after the first time, so it starts straight away.
                    let dir = format!("{}/Videos", home());
                    let _ = std::fs::create_dir_all(&dir);
                    let file = format!("{dir}/Screen Recording {}.mp4", stamp());
                    let token = format!("{}/.config/cosmic-mac/recorder-portal-token", home());
                    let mut args: Vec<String> = cmd[1..].to_vec();
                    args.extend(["-w", "portal", "-f", "60", "-cursor", "yes", "-o"].map(String::from));
                    args.push(file.clone());
                    if !choose {
                        args.extend(["-restore-portal-session", "yes", "-portal-session-token-filepath"].map(String::from));
                        args.push(token);
                    }
                    if let Ok(mut child) = Command::new(&cmd[0]).args(&args).spawn() {
                        self.status.recording = true;
                        std::thread::spawn(move || {
                            let ok = child.wait().is_ok_and(|s| s.success() || s.code() == Some(130));
                            if ok && std::path::Path::new(&file).exists() {
                                let _ = Command::new("notify-send")
                                    .args(["-a", "Screen Recording", "-i", "media-record-symbolic", "Screen recording saved", &file])
                                    .status();
                            }
                        });
                    }
                    return self.close_popup();
                } else {
                    run("cosmic-store", &[]);
                    return self.close_popup();
                }
            }
            Message::Open(page) => {
                run("cosmic-settings", &[page]);
                return self.close_popup();
            }
            Message::SetBarIcon(icon) => {
                write_bar_icon(icon.as_deref().filter(|i| *i != BAR_ICONS[0].0));
                self.bar_icon = icon;
            }
            Message::CustomIcon(v) => self.custom_icon = v,
            Message::ApplyCustomIcon => {
                let v = self.custom_icon.trim().to_string();
                if !v.is_empty() {
                    return self.update(Message::SetBarIcon(Some(v)));
                }
            }
            Message::ToggleMeter(m) => {
                if let Some(i) = self.meters.iter().position(|x| x == m) {
                    if self.meters.len() > 1 {
                        self.meters.remove(i);
                    }
                } else {
                    // keep the canonical left-to-right order
                    let mut next: Vec<String> = METERS.iter().map(|(k, _)| k.to_string()).filter(|k| k == m || self.meters.contains(k)).collect();
                    next.dedup();
                    self.meters = next;
                }
                write_meters(&self.meters);
            }
            Message::EditWidgets => {
                let path = format!("{}/.config/cosmic-mac/widgets.conf", home());
                run("cosmic-edit", &[&path]);
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
        // While recording, the icon becomes a red record dot (click it to open the stop tiles).
        if self.status.recording {
            let (size, _) = self.core.applet.suggested_size(true);
            let dot = container(icon::from_name("media-record-symbolic").size(size).symbolic(true)).class(theme::Container::custom(|_| {
                cosmic::iced::widget::container::Style { icon_color: Some(Color::from_rgb(1.0, 0.231, 0.188)), ..Default::default() }
            }));
            return self
                .core
                .applet
                .autosize_window(button::custom(dot).class(Button::AppletIcon).padding(style::bar_padding(&self.core.applet)).on_press_down(Message::TogglePopup))
                .into();
        }
        self.core.applet.icon_button("com.aqib.control-center-symbolic").on_press_down(Message::TogglePopup).into()
    }

    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        // Wi-Fi / Bluetooth detail pages replace the grid, with a back arrow, like macOS.
        match self.page {
            Page::Wifi => {
                let menu = menus::wifi(
                    &self.wifi,
                    self.show_other,
                    self.connecting.as_deref(),
                    menus::WifiMsgs {
                        toggle: Message::WifiEnable,
                        connect: Message::WifiConnect,
                        other: Message::WifiOther,
                        settings: Message::Open("wireless"),
                        back: Some(Message::ShowPage(Page::Main)),
                    },
                );
                return style::popover(Column::new().push(menu).padding([10, 0]), WIDTH);
            }
            Page::Bluetooth => {
                let menu = menus::bluetooth(
                    &self.bt,
                    self.show_nearby,
                    self.busy.as_deref(),
                    menus::BtMsgs {
                        toggle: Message::BtEnable,
                        device: Message::BtDevice,
                        nearby: Message::BtNearby,
                        settings: Message::Open("bluetooth"),
                        back: Some(Message::ShowPage(Page::Main)),
                    },
                );
                return style::popover(Column::new().push(menu).padding([10, 0]), WIDTH);
            }
            Page::Customize => return self.customize_view(),
            Page::Main => {}
        }
        let s = &self.status;
        let on_off = |on: bool| Some(if on { "On" } else { "Off" }.to_string());

        // Connectivity pills + Now Playing
        let wifi_sub = if s.wifi_on { s.wifi_name.clone().unwrap_or_else(|| "Not Connected".into()) } else { "Off".into() };
        let conn = Column::new()
            .push(split_pill("network-wireless-signal-excellent-symbolic", "Wi-Fi", wifi_sub, s.wifi_on, Message::Wifi, Message::ShowPage(Page::Wifi)))
            .push(split_pill(
                "bluetooth-active-symbolic",
                "Bluetooth",
                if s.bluetooth_on { "On" } else { "Off" }.to_string(),
                s.bluetooth_on,
                Message::Bluetooth,
                Message::ShowPage(Page::Bluetooth),
            ))
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
        if let Some(t) = &s.touch {
            let sub = if !t.writable { "Set Up…" } else if t.on { "On" } else { "Off" };
            grid = grid.push(row![pill("input-tablet-symbolic", "Touch", Some(sub.to_string()), t.on, Message::Touch)].spacing(GAP));
        }

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
                    pill("selection-mode-symbolic", "Record Selection", rec_sub.clone(), s.recording, Message::Record { choose: true }),
                    pill("applets-screenshooter-symbolic", "Capture", None, false, Message::Screenshot),
                ]
                .spacing(GAP),
            )
            .push(
                row![
                    pill("media-record-symbolic", "Record Screen", rec_sub, s.recording, Message::Record { choose: false }),
                    pill("camera-video-symbolic", "Record", on_off(s.recording), s.recording, Message::Record { choose: false }),
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
            .push(button::custom(text::body("Edit Controls")).class(tile_button(999.0, false)).padding([6, 14]).on_press(Message::ShowPage(Page::Customize)))
            .spacing(8)
            .align_y(Alignment::Center);

        let content = Column::new().push(grid).push(footer).spacing(14).padding(PAD);
        style::popover(content, WIDTH)
    }

    fn subscription(&self) -> Subscription<Message> {
        let open = time::every(Duration::from_millis(500)).map(|_| Message::CheckOpen);
        if self.popup.is_some() || self.status.recording {
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
