//! macOS-style system monitor for the COSMIC panel.
//!
//! Panel: icon + value for each chosen meter, e.g. [cpu] 12%  [ram] 38%.
//! Popover (macOS menu styling): CPU with top processes, Memory, Network, Disk,
//! then "Activity Monitor…".
//! Settings: ~/.config/cosmic-mac/stats.conf  (SHOW=cpu,memory,network,disk).
//! Reads /proc and statvfs directly; nothing runs in the background but a 2 s tick.

#[path = "../../shared/style.rs"]
mod style;

use std::{collections::HashMap, time::Duration};

use cosmic::{
    Element, Task, app,
    iced::{Alignment, Background, Border, Color, Length, Subscription, time, window},
    surface,
    theme::{self, Button},
    widget::{Column, Row, button, container, icon, row, space, text},
};

const APP_ID: &str = "com.aqib.CosmicMacStats";

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<Stats>(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Meter {
    Cpu,
    Memory,
    Network,
    Disk,
}

fn load_show() -> Vec<Meter> {
    let path = format!("{}/.config/cosmic-mac/stats.conf", std::env::var("HOME").unwrap_or_default());
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        let default = "# Meters shown in the top bar, left to right: cpu, memory, network, disk\nSHOW=cpu,memory\n";
        let _ = std::fs::write(&path, default);
        default.to_string()
    });
    let list = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("SHOW="))
        .unwrap_or("cpu,memory")
        .split(',')
        .filter_map(|m| match m.trim() {
            "cpu" => Some(Meter::Cpu),
            "memory" | "mem" | "ram" => Some(Meter::Memory),
            "network" | "net" => Some(Meter::Network),
            "disk" => Some(Meter::Disk),
            _ => None,
        })
        .collect::<Vec<_>>();
    if list.is_empty() { vec![Meter::Cpu, Meter::Memory] } else { list }
}

#[derive(Default)]
struct Sampler {
    cpu_prev: Option<(u64, u64)>, // (busy, total)
    net_prev: Option<(u64, u64, std::time::Instant)>,
    proc_prev: HashMap<u32, u64>,
    proc_total_prev: u64,
}

#[derive(Clone, Debug, Default)]
struct Snapshot {
    cpu: f32, // 0..100
    mem_used: u64,
    mem_total: u64,
    swap_used: u64,
    swap_total: u64,
    down: f64, // bytes/s
    up: f64,
    disk_used: u64,
    disk_total: u64,
    top: Vec<(String, f32)>,
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn cpu_times() -> Option<(u64, u64)> {
    let stat = read("/proc/stat");
    let v: Vec<u64> = stat.lines().next()?.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    let idle = v.get(3)? + v.get(4).unwrap_or(&0);
    let total: u64 = v.iter().take(8).sum();
    Some((total - idle, total))
}

fn meminfo() -> HashMap<String, u64> {
    read("/proc/meminfo")
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((k.to_string(), v.split_whitespace().next()?.parse::<u64>().ok()? * 1024))
        })
        .collect()
}

fn net_bytes() -> (u64, u64) {
    let mut rx = 0;
    let mut tx = 0;
    for line in read("/proc/net/dev").lines().skip(2) {
        let Some((iface, rest)) = line.split_once(':') else { continue };
        let iface = iface.trim();
        if iface == "lo" || iface.starts_with("veth") || iface.starts_with("docker") || iface.starts_with("br-") {
            continue;
        }
        let f: Vec<u64> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        rx += f.first().copied().unwrap_or(0);
        tx += f.get(8).copied().unwrap_or(0);
    }
    (rx, tx)
}

fn disk() -> (u64, u64) {
    // `df -B1 /` → used, size
    let out = std::process::Command::new("df").args(["-B1", "--output=used,size", "/"]).output();
    let text = out.map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    let v: Vec<u64> = text.lines().nth(1).unwrap_or("").split_whitespace().filter_map(|x| x.parse().ok()).collect();
    (v.first().copied().unwrap_or(0), v.get(1).copied().unwrap_or(0))
}

fn process_ticks() -> HashMap<u32, u64> {
    let mut map = HashMap::new();
    let Ok(dir) = std::fs::read_dir("/proc") else { return map };
    for e in dir.flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        let stat = read(&format!("/proc/{pid}/stat"));
        let Some(after) = stat.rsplit_once(')').map(|(_, a)| a) else { continue };
        let f: Vec<&str> = after.split_whitespace().collect();
        if let (Some(u), Some(s)) = (f.get(11).and_then(|x| x.parse::<u64>().ok()), f.get(12).and_then(|x| x.parse::<u64>().ok())) {
            map.insert(pid, u + s);
        }
    }
    map
}

impl Sampler {
    fn sample(&mut self, with_processes: bool) -> Snapshot {
        let mut s = Snapshot::default();
        if let Some((busy, total)) = cpu_times() {
            if let Some((pb, pt)) = self.cpu_prev {
                let dt = total.saturating_sub(pt).max(1) as f32;
                s.cpu = (busy.saturating_sub(pb) as f32 / dt * 100.0).clamp(0.0, 100.0);
            }
            self.cpu_prev = Some((busy, total));

            if with_processes {
                let now = process_ticks();
                let elapsed = total.saturating_sub(self.proc_total_prev) as f32;
                if self.proc_total_prev > 0 && elapsed > 0.0 {
                    let mut by_name: HashMap<String, f32> = HashMap::new();
                    for (pid, ticks) in &now {
                        let Some(before) = self.proc_prev.get(pid) else { continue };
                        // share of the whole machine (all cores), like the CPU meter above
                        let pct = ticks.saturating_sub(*before) as f32 / elapsed * 100.0;
                        if pct > 0.05 {
                            let name = read(&format!("/proc/{pid}/comm")).trim().to_string();
                            *by_name.entry(name).or_default() += pct;
                        }
                    }
                    let mut top: Vec<(String, f32)> = by_name.into_iter().collect();
                    top.sort_by(|a, b| b.1.total_cmp(&a.1));
                    top.truncate(4);
                    s.top = top;
                }
                self.proc_prev = now;
                self.proc_total_prev = total;
            }
        }
        let m = meminfo();
        s.mem_total = m.get("MemTotal").copied().unwrap_or(0);
        s.mem_used = s.mem_total.saturating_sub(m.get("MemAvailable").copied().unwrap_or(0));
        s.swap_total = m.get("SwapTotal").copied().unwrap_or(0);
        s.swap_used = s.swap_total.saturating_sub(m.get("SwapFree").copied().unwrap_or(0));
        let (rx, tx) = net_bytes();
        let now = std::time::Instant::now();
        if let Some((prx, ptx, at)) = self.net_prev {
            let secs = now.duration_since(at).as_secs_f64().max(0.1);
            s.down = rx.saturating_sub(prx) as f64 / secs;
            s.up = tx.saturating_sub(ptx) as f64 / secs;
        }
        self.net_prev = Some((rx, tx, now));
        (s.disk_used, s.disk_total) = disk();
        s
    }
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

fn rate(bps: f64) -> String {
    match bps {
        b if b >= 1e6 => format!("{:.1} MB/s", b / 1e6),
        b if b >= 1e3 => format!("{:.0} KB/s", b / 1e3),
        b => format!("{:.0} B/s", b),
    }
}

fn pct(used: u64, total: u64) -> f32 {
    if total == 0 { 0.0 } else { used as f32 / total as f32 * 100.0 }
}

/// A thin macOS-style level bar.
fn meter<'a, M: 'static>(percent: f32) -> Element<'a, M> {
    let p = (percent.round() as u16).clamp(0, 100);
    let fill = container(space::horizontal())
        .height(Length::Fixed(6.0))
        .width(Length::FillPortion(p.max(1)))
        .class(theme::Container::custom(|_| cosmic::iced::widget::container::Style {
            background: Some(Background::Color(style::MAC_BLUE)),
            border: Border { radius: 3.0.into(), ..Default::default() },
            ..Default::default()
        }));
    let rest = space::horizontal().width(Length::FillPortion((100 - p).max(1)));
    container(row![fill, rest])
        .height(Length::Fixed(6.0))
        .width(Length::Fill)
        .class(theme::Container::custom(|t| cosmic::iced::widget::container::Style {
            background: Some(Background::Color(if t.cosmic().is_dark { Color::from_rgba(1.0, 1.0, 1.0, 0.12) } else { Color::from_rgba(0.0, 0.0, 0.0, 0.08) })),
            border: Border { radius: 3.0.into(), ..Default::default() },
            ..Default::default()
        }))
        .into()
}

#[derive(Default)]
struct Stats {
    core: app::Core,
    popup: Option<window::Id>,
    sampler: Sampler,
    snap: Snapshot,
    show: Vec<Meter>,
}

#[derive(Debug, Clone)]
enum Message {
    TogglePopup,
    CloseRequested(window::Id),
    Tick,
    CheckOpen,
    OpenMonitor,
}

impl Stats {
    fn close_popup(&mut self) -> app::Task<Message> {
        match self.popup.take() {
            Some(p) => surface::surface_task(surface::action::destroy_popup(p)),
            None => Task::none(),
        }
    }

    fn bar_value(&self, m: Meter) -> (&'static str, String) {
        let s = &self.snap;
        match m {
            Meter::Cpu => ("cpu-symbolic", format!("{:.0}%", s.cpu)),
            Meter::Memory => ("ram-symbolic", format!("{:.0}%", pct(s.mem_used, s.mem_total))),
            Meter::Network => ("go-down-symbolic", rate(s.down)),
            Meter::Disk => ("drive-harddisk-symbolic", format!("{:.0}%", pct(s.disk_used, s.disk_total))),
        }
    }
}

impl cosmic::Application for Stats {
    type Message = Message;
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    const APP_ID: &'static str = APP_ID;

    fn init(core: app::Core, _flags: ()) -> (Self, app::Task<Message>) {
        let mut app = Self { core, show: load_show(), ..Default::default() };
        app.snap = app.sampler.sample(false);
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
            Message::TogglePopup => {
                if self.popup.is_some() {
                    return self.close_popup();
                }
                self.show = load_show();
                self.sampler.sample(true); // prime the per-process counters
                let new_id = window::Id::unique();
                return surface::surface_task(surface::action::app_popup(
                    |_| Default::default(),
                    move |app: &mut Stats| {
                        app.popup.replace(new_id);
                        app.core.applet.get_popup_settings(app.core.main_window_id().unwrap(), new_id, Some((1, 1)), None, None)
                    },
                    None,
                )).chain(style::blur(new_id)).chain(style::blur_soon(new_id, 150)).chain(style::blur_soon(new_id, 700));
            }
            Message::CloseRequested(id) => {
                if Some(id) == self.popup {
                    self.popup = None;
                }
            }
            Message::Tick => {
                self.snap = self.sampler.sample(self.popup.is_some());
                self.show = load_show(); // picks up changes made in Control Center → Edit Controls
            }
            Message::CheckOpen => {
                if style::open_requested(APP_ID) {
                    return self.update(Message::TogglePopup);
                }
            }
            Message::OpenMonitor => {
                if let Ok(mut c) = std::process::Command::new("gnome-system-monitor").spawn() {
                    std::thread::spawn(move || {
                        let _ = c.wait();
                    });
                }
                return self.close_popup();
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let (size, _) = self.core.applet.suggested_size(true);
        let mut r = Row::new().spacing(10).align_y(Alignment::Center);
        for m in &self.show {
            let (glyph, value) = self.bar_value(*m);
            r = r.push(
                row![icon::from_name(glyph).size(size).symbolic(true), self.core.applet.text(value)]
                    .spacing(3)
                    .align_y(Alignment::Center),
            );
        }
        self.core
            .applet
            .autosize_window(button::custom(r).class(Button::AppletIcon).padding(style::bar_padding(&self.core.applet)).on_press_down(Message::TogglePopup))
            .into()
    }

    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let s = &self.snap;
        let item = |c: Element<'static, Message>| container(c).padding([4, 14]).width(Length::Fill);
        let title = |name: &'static str, glyph: &'static str, value: String| -> Element<'static, Message> {
            row![
                icon::from_name(glyph).size(16).symbolic(true),
                text::heading(name).width(Length::Fill),
                text::heading(value),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        };

        let mut cpu = Column::new()
            .push(title("CPU", "cpu-symbolic", format!("{:.0}%", s.cpu)))
            .push(meter(s.cpu))
            .spacing(6);
        if !s.top.is_empty() {
            cpu = cpu.push(style::section("Top Processes"));
            for (name, p) in &s.top {
                cpu = cpu.push(row![text::body(name.clone()).width(Length::Fill), style::secondary(format!("{p:.1}%"))]);
            }
        }

        let mut mem = Column::new()
            .push(title("Memory", "ram-symbolic", format!("{:.0}%", pct(s.mem_used, s.mem_total))))
            .push(meter(pct(s.mem_used, s.mem_total)))
            .push(style::secondary(format!("{} of {} used", gb(s.mem_used), gb(s.mem_total))))
            .spacing(6);
        if s.swap_total > 0 {
            mem = mem.push(style::secondary(format!("Swap: {} of {}", gb(s.swap_used), gb(s.swap_total))));
        }

        let net = Column::new()
            .push(title("Network", "network-transmit-receive-symbolic", String::new()))
            .push(row![
                icon::from_name("go-down-symbolic").size(14).symbolic(true),
                text::body(format!(" {}", rate(s.down))).width(Length::Fill),
                icon::from_name("go-up-symbolic").size(14).symbolic(true),
                text::body(format!(" {}", rate(s.up))).width(Length::Fill),
            ])
            .spacing(6);

        let disk = Column::new()
            .push(title("Disk", "drive-harddisk-symbolic", format!("{:.0}%", pct(s.disk_used, s.disk_total))))
            .push(meter(pct(s.disk_used, s.disk_total)))
            .push(style::secondary(format!("{} of {} used", gb(s.disk_used), gb(s.disk_total))))
            .spacing(6);

        let content = Column::new()
            .push(item(cpu.into()))
            .push(style::separator())
            .push(item(mem.into()))
            .push(style::separator())
            .push(item(net.into()))
            .push(style::separator())
            .push(item(disk.into()))
            .push(style::separator())
            .push(button::custom(text::body("Activity Monitor…")).class(Button::AppletMenu).padding([4, 14]).width(Length::Fill).on_press(Message::OpenMonitor))
            .spacing(4)
            .padding([10, 0]);
        style::popover(content, 300.0)
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            time::every(Duration::from_secs(2)).map(|_| Message::Tick),
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
