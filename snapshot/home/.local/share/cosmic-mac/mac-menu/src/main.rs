//! macOS menu bar, left side: the logo menu and the focused app's name in bold.
//!
//! Logo menu: About This Computer, System Settings…, App Store…, Force Quit…, Sleep,
//! Restart…, Shut Down…, Lock Screen, Log Out…
//! App menu: Hide <App>, Hide Others, Quit <App> (done by cosmic-maximize-to-workspace --app).
//! The focused app comes from $XDG_RUNTIME_DIR/cosmic-mac/focused, written by the
//! maximize helper. The logo is the icon chosen in Control Center → Edit Controls.

#[path = "../../shared/style.rs"]
mod style;

use std::{collections::HashMap, time::Duration};

use cosmic::{
    Element, Task, app,
    iced::{Alignment, Length, Subscription, time, window},
    surface,
    theme::Button,
    widget::{Column, button, icon, row, text},
};

const APP_ID: &str = "com.aqib.CosmicMacMenu";
const DESKTOP_NAME: &str = "Files";

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<MacMenu>(())
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

/// The icon chosen for the left-most bar button in Control Center → Edit Controls.
fn logo() -> String {
    let conf = format!("{}/.config/cosmic/com.system76.CosmicPanelButton/v1/configs", home());
    std::fs::read_to_string(conf)
        .ok()
        .and_then(|t| {
            let line = t.lines().find(|l| l.contains("\"*/com.system76.CosmicAppLibrary\""))?.to_string();
            let rest = line.split("icon: Some(\"").nth(1)?;
            Some(rest.split('"').next()?.to_string())
        })
        .unwrap_or_else(|| "distributor-logo-fedora".to_string())
}

fn focused() -> (String, String) {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    let t = std::fs::read_to_string(format!("{dir}/cosmic-mac/focused")).unwrap_or_default();
    let mut l = t.lines();
    (l.next().unwrap_or("").to_string(), l.next().unwrap_or("").to_string())
}

/// "Name=" of the app's .desktop file, else a tidied app id.
fn app_name(app_id: &str) -> String {
    let dirs = [
        format!("{}/.local/share/applications", home()),
        format!("{}/.local/share/flatpak/exports/share/applications", home()),
        "/var/lib/flatpak/exports/share/applications".to_string(),
        "/usr/share/applications".to_string(),
    ];
    let wanted = app_id.to_lowercase();
    for d in &dirs {
        let Ok(rd) = std::fs::read_dir(d) else { continue };
        for e in rd.flatten() {
            let file = e.file_name().to_string_lossy().to_lowercase();
            let Some(stem) = file.strip_suffix(".desktop") else { continue };
            let text = std::fs::read_to_string(e.path()).unwrap_or_default();
            let wm = text
                .lines()
                .find_map(|l| l.strip_prefix("StartupWMClass="))
                .map(|w| w.trim().to_lowercase());
            if stem == wanted || wm.as_deref() == Some(wanted.as_str()) || stem.rsplit('.').next() == Some(wanted.as_str()) {
                if let Some(n) = text.lines().find_map(|l| l.strip_prefix("Name=")) {
                    return n.trim().to_string();
                }
            }
        }
    }
    let last = app_id.rsplit('.').next().unwrap_or(app_id);
    let mut c = last.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn spawn(cmd: &str, args: &[&str]) {
    if let Ok(mut c) = std::process::Command::new(cmd).args(args).spawn() {
        std::thread::spawn(move || {
            let _ = c.wait();
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Which {
    Logo,
    App,
}

#[derive(Debug, Clone)]
enum Message {
    Toggle(Which),
    CloseRequested(window::Id),
    Tick,
    Run(&'static str, Vec<String>),
    AppAction(&'static str),
}

#[derive(Default)]
struct MacMenu {
    core: app::Core,
    popup: Option<(window::Id, Which)>,
    logo: String,
    app_id: String,
    name: String,
    names: HashMap<String, String>,
}

impl MacMenu {
    fn close(&mut self) -> app::Task<Message> {
        match self.popup.take() {
            Some((p, _)) => surface::surface_task(surface::action::destroy_popup(p)),
            None => Task::none(),
        }
    }

    fn refresh(&mut self) {
        self.logo = logo();
        // keep the app the menu was opened for while it is open
        if self.popup.is_some() {
            return;
        }
        let (id, _title) = focused();
        if id != self.app_id {
            self.name = if id.is_empty() {
                DESKTOP_NAME.to_string()
            } else {
                self.names.entry(id.clone()).or_insert_with(|| app_name(&id)).clone()
            };
            self.app_id = id;
        }
    }
}

impl cosmic::Application for MacMenu {
    type Message = Message;
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    const APP_ID: &'static str = APP_ID;

    fn init(core: app::Core, _flags: ()) -> (Self, app::Task<Message>) {
        let mut app = Self { core, ..Default::default() };
        app.refresh();
        (app, Task::none())
    }

    fn core(&self) -> &app::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut app::Core {
        &mut self.core
    }

    fn update(&mut self, message: Message) -> app::Task<Message> {
        match message {
            Message::Toggle(which) => {
                let was = self.popup.map(|p| p.1);
                let close = self.close();
                if was == Some(which) {
                    return close;
                }
                let open = surface::surface_task(surface::action::app_popup(
                    |_| Default::default(),
                    move |app: &mut MacMenu| {
                        let new_id = window::Id::unique();
                        app.popup = Some((new_id, which));
                        let mut s = app.core.applet.get_popup_settings(app.core.main_window_id().unwrap(), new_id, Some((1, 1)), None, None);
                        // open under the button that was clicked, left-aligned like macOS
                        if which == Which::App {
                            s.positioner.anchor_rect.x += app.core.applet.suggested_size(true).0 as i32 + 8;
                        }
                        s
                    },
                    None,
                ));
                return close.chain(open);
            }
            Message::CloseRequested(id) => {
                if self.popup.is_some_and(|p| p.0 == id) {
                    self.popup = None;
                }
            }
            Message::Tick => self.refresh(),
            Message::Run(cmd, args) => {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                spawn(cmd, &args);
                return self.close();
            }
            Message::AppAction(action) => {
                if self.app_id.is_empty() {
                    spawn("nautilus", &["--new-window"]);
                } else {
                    spawn("cosmic-maximize-to-workspace", &["--app", action, &self.app_id]);
                }
                return self.close();
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let (size, _) = self.core.applet.suggested_size(true);
        let logo_icon = if self.logo.starts_with('/') {
            icon::from_path(self.logo.clone().into())
        } else {
            icon::from_name(self.logo.clone()).handle()
        };
        let logo = button::custom(icon::icon(logo_icon).size(size))
            .class(Button::AppletIcon)
            .padding(style::bar_padding(&self.core.applet))
            .on_press_down(Message::Toggle(Which::Logo));
        let name = button::custom(self.core.applet.text(self.name.clone()).font(cosmic::font::bold()))
            .class(Button::AppletIcon)
            .padding(style::bar_padding(&self.core.applet))
            .on_press_down(Message::Toggle(Which::App));
        self.core
            .applet
            .autosize_window(row![logo, name].spacing(2).align_y(Alignment::Center))
            .into()
    }

    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let item = |label: String, msg: Message| -> Element<'static, Message> {
            button::custom(text::body(label))
                .class(Button::AppletMenu)
                .padding([4, 14])
                .width(Length::Fill)
                .on_press(msg)
                .into()
        };
        let run = |label: &str, cmd: &'static str, args: &[&str]| {
            item(label.to_string(), Message::Run(cmd, args.iter().map(|a| a.to_string()).collect()))
        };
        let content: Column<'_, Message, cosmic::Theme, cosmic::Renderer> = match self.popup.map(|p| p.1) {
            Some(Which::App) if !self.app_id.is_empty() => Column::new()
                .push(item(format!("Hide {}", self.name), Message::AppAction("hide")))
                .push(item("Hide Others".to_string(), Message::AppAction("hide-others")))
                .push(style::separator())
                .push(item(format!("Quit {}", self.name), Message::AppAction("quit"))),
            Some(Which::App) => Column::new()
                .push(item("New Files Window".to_string(), Message::AppAction("open")))
                .push(style::separator())
                .push(run("Hide Others", "cosmic-maximize-to-workspace", &["--app", "hide-others", ""])),
            _ => Column::new()
                .push(run("About This Computer", "cosmic-settings", &["about"]))
                .push(style::separator())
                .push(run("System Settings…", "cosmic-settings", &[]))
                .push(run("App Store…", "cosmic-store", &[]))
                .push(style::separator())
                .push(run("Force Quit…", "gnome-system-monitor", &["-p"]))
                .push(style::separator())
                .push(run("Sleep", "systemctl", &["suspend"]))
                .push(run("Restart…", "cosmic-osd", &["restart"]))
                .push(run("Shut Down…", "cosmic-osd", &["shutdown"]))
                .push(style::separator())
                .push(run("Lock Screen", "loginctl", &["lock-session"]))
                .push(run("Log Out…", "cosmic-osd", &["log-out"])),
        };
        style::popover(content.spacing(2).padding([6, 0]), 240.0)
    }

    fn subscription(&self) -> Subscription<Message> {
        time::every(Duration::from_millis(300)).map(|_| Message::Tick)
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::CloseRequested(id))
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
