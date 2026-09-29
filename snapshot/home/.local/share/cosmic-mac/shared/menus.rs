//! The macOS Wi-Fi and Bluetooth menus, drawn the same way in their own top-bar
//! items and inside the Control Center. Shared by the cosmic-mac applets.
#![allow(dead_code)]

use cosmic::{
    Element,
    iced::{Alignment, Length},
    theme::Button,
    widget::{Column, button, container, icon, row, text, toggler},
};

use crate::{bt, net, style};

fn item<'a, M: 'static>(content: impl Into<Element<'a, M>>) -> Element<'a, M> {
    container(content).padding([4, 14]).width(Length::Fill).into()
}

fn menu_row<'a, M: Clone + 'static>(content: impl Into<Element<'a, M>>, msg: Option<M>) -> Element<'a, M> {
    let mut b = button::custom(content).class(Button::AppletMenu).padding([3, 14]).width(Length::Fill);
    if let Some(m) = msg {
        b = b.on_press(m);
    }
    b.into()
}

fn badge<'a, M: 'static>(icon_name: &'static str, active: bool) -> Element<'a, M> {
    container(icon::from_name(icon_name).size(16).symbolic(true))
        .width(Length::Fixed(28.0))
        .height(Length::Fixed(28.0))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .class(style::badge(active))
        .into()
}

/// Title row: optional back chevron (inside the Control Center), title, switch.
fn header<'a, M: Clone + 'static>(title: &'static str, on: bool, toggle: impl Fn(bool) -> M + 'a, back: Option<M>) -> Element<'a, M> {
    let mut r = row![].align_y(Alignment::Center).spacing(6);
    if let Some(b) = back {
        r = r.push(button::custom(icon::from_name("go-previous-symbolic").size(16).symbolic(true)).class(Button::Icon).padding(2).on_press(b));
    }
    item(r.push(text::heading(title).size(15).width(Length::Fill)).push(toggler(on).on_toggle(toggle)))
}

fn expander<'a, M: Clone + 'static>(title: &'static str, open: bool, msg: M) -> Element<'a, M> {
    menu_row(
        row![
            text::heading(title).width(Length::Fill),
            icon::from_name(if open { "go-down-symbolic" } else { "go-next-symbolic" }).size(14).symbolic(true),
        ]
        .align_y(Alignment::Center),
        Some(msg),
    )
}

pub struct WifiMsgs<M> {
    pub toggle: fn(bool) -> M,
    pub connect: fn(net::Network) -> M,
    pub other: M,
    pub settings: M,
    pub back: Option<M>,
}

pub fn wifi<'a, M: Clone + 'static>(s: &'a net::State, show_other: bool, connecting: Option<&'a str>, m: WifiMsgs<M>) -> Element<'a, M> {
    let net_row = |n: &'a net::Network| {
        let label = if connecting == Some(n.ssid.as_str()) { format!("{}  (connecting…)", n.ssid) } else { n.ssid.clone() };
        let mut r = row![badge(net::signal_icon(n.signal), n.in_use), text::body(label).width(Length::Fill)].spacing(10).align_y(Alignment::Center);
        if n.secured() {
            r = r.push(icon::from_name("system-lock-screen-symbolic").size(14).symbolic(true));
        }
        menu_row(r, Some((m.connect)(n.clone())))
    };
    let mut col = Column::new().push(header("Wi-Fi", s.enabled, m.toggle, m.back.clone()));
    if s.enabled {
        if s.current().is_some_and(net::Network::weak) {
            col = col.push(menu_row(
                row![
                    text::body("Weak Security…").width(Length::Fill),
                    container(icon::from_name("dialog-warning-symbolic").size(16).symbolic(true)).class(style::low_power_tint(true)),
                ]
                .align_y(Alignment::Center),
                Some(m.settings.clone()),
            ));
        }
        let known: Vec<&net::Network> = s.networks.iter().filter(|n| n.known || n.in_use).collect();
        if !known.is_empty() {
            col = col.push(style::separator()).push(item(style::section("Known Networks")));
            for n in known {
                col = col.push(net_row(n));
            }
        }
        col = col.push(style::separator()).push(expander("Other Networks", show_other, m.other.clone()));
        if show_other {
            let others: Vec<&net::Network> = s.networks.iter().filter(|n| !n.known && !n.in_use).collect();
            if others.is_empty() {
                col = col.push(item(style::secondary("Searching…")));
            }
            for n in others {
                col = col.push(net_row(n));
            }
        }
    }
    col.push(style::separator()).push(menu_row(text::body("Wi-Fi Settings…"), Some(m.settings))).spacing(2).into()
}

pub struct BtMsgs<M> {
    pub toggle: fn(bool) -> M,
    pub device: fn(bt::Device) -> M,
    pub nearby: M,
    pub settings: M,
    pub back: Option<M>,
}

pub fn bluetooth<'a, M: Clone + 'static>(s: &'a bt::State, show_nearby: bool, busy: Option<&'a str>, m: BtMsgs<M>) -> Element<'a, M> {
    let dev_row = |d: &'a bt::Device| {
        let label = if busy == Some(d.mac.as_str()) {
            format!("{}  ({}…)", d.name, if d.connected { "disconnecting" } else { "connecting" })
        } else {
            d.name.clone()
        };
        let mut r = row![badge(bt::device_icon(&d.icon), d.connected), text::body(label).width(Length::Fill)].spacing(10).align_y(Alignment::Center);
        if let Some(pct) = d.battery {
            r = r.push(style::secondary(format!("{pct}%")));
        }
        menu_row(r, Some((m.device)(d.clone())))
    };
    let mut col = Column::new().push(header("Bluetooth", s.powered, m.toggle, m.back.clone()));
    if !s.available {
        col = col.push(item(style::secondary("No Bluetooth adapter found")));
    } else if s.powered {
        let paired: Vec<&bt::Device> = s.devices.iter().filter(|d| d.paired).collect();
        col = col.push(style::separator()).push(item(style::section("Devices")));
        if paired.is_empty() {
            col = col.push(item(style::secondary("No paired devices")));
        }
        for d in paired {
            col = col.push(dev_row(d));
        }
        col = col.push(style::separator()).push(expander("Nearby Devices", show_nearby, m.nearby.clone()));
        if show_nearby {
            let near: Vec<&bt::Device> = s.devices.iter().filter(|d| !d.paired).collect();
            if near.is_empty() {
                col = col.push(item(style::secondary("Searching…")));
            }
            for d in near {
                col = col.push(dev_row(d));
            }
        }
    }
    col.push(style::separator()).push(menu_row(text::body("Bluetooth Settings…"), Some(m.settings))).spacing(2).into()
}
