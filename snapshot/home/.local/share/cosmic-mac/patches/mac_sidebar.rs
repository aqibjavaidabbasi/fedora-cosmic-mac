//! macOS System Settings sidebar for COSMIC Settings (cosmic-mac patches/settings.py).
//!
//! A flat list like macOS: the search field on top, then groups of pages, each with a
//! colored rounded-square icon; the selected page is a blue pill. Pages not listed here
//! stay reachable through search and their parent pages.

use cosmic::{
    Element,
    iced::{Alignment, Background, Border, Color, Length},
    theme,
    widget::{button, column, container, icon, row, scrollable, text, text_input},
};

use super::{Message, SettingsApp};

/// (page id, label override, icon background)
type Entry = (&'static str, &'static str, u32);

const GROUPS: &[&[Entry]] = &[
    &[
        ("wifi", "Wi-Fi", 0x0A84FF),
        ("bluetooth", "", 0x0A84FF),
        ("network-and-wireless", "Network", 0x0A84FF),
        ("vpn", "VPN", 0x0A84FF),
    ],
    &[("sound", "", 0xFF375F)],
    &[
        ("about", "", 0x8E8E93),
        ("appearance", "", 0x48484A),
        ("accessibility", "", 0x0A84FF),
    ],
    &[
        ("dock", "Desktop & Dock", 0x48484A),
        ("panel", "Menu Bar", 0x48484A),
        ("display", "Displays", 0x0A84FF),
        ("wallpaper", "", 0x32ADE6),
        ("workspaces", "", 0x5E5CE6),
        ("window-management", "", 0x5E5CE6),
        ("power", "Battery", 0x34C759),
    ],
    &[("users", "Users & Groups", 0x0A84FF)],
    &[
        ("keyboard", "", 0x8E8E93),
        ("mouse", "", 0x8E8E93),
        ("touchpad", "Trackpad", 0x8E8E93),
        ("time-date", "", 0x0A84FF),
        ("time-region", "", 0x0A84FF),
        ("default-apps", "", 0x8E8E93),
        ("startup-apps", "Login Items", 0x8E8E93),
    ],
];

fn rgb(hex: u32) -> Color {
    Color::from_rgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// White glyph on a 24px rounded square, like macOS sidebar icons.
fn squircle<'a>(glyph: &str, hex: u32) -> Element<'a, Message> {
    let bg = rgb(hex);
    container(icon::from_name(glyph.to_string()).size(15).symbolic(true))
        .width(Length::Fixed(24.0))
        .height(Length::Fixed(24.0))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .class(theme::Container::custom(move |_| container::Style {
            background: Some(Background::Color(bg)),
            border: Border { radius: 6.0.into(), ..Default::default() },
            icon_color: Some(Color::WHITE),
            text_color: Some(Color::WHITE),
            ..Default::default()
        }))
        .into()
}

fn row_style(selected: bool, hover: bool) -> theme::Button {
    let make = move |pressed: bool| {
        move |_focused: bool, t: &cosmic::Theme| {
            let c = t.cosmic();
            let on: Color = c.on_bg_color().into();
            let (bg, fg) = if selected {
                (Some(Background::Color(rgb(0x0A84FF))), Color::WHITE)
            } else if hover || pressed {
                (Some(Background::Color(Color { a: if pressed { 0.12 } else { 0.07 }, ..on })), on)
            } else {
                (None, on)
            };
            cosmic::widget::button::Style {
                background: bg,
                border_radius: 7.0.into(),
                text_color: Some(fg),
                icon_color: Some(fg),
                ..cosmic::widget::button::Style::new()
            }
        }
    };
    theme::Button::Custom {
        active: Box::new(make(false)),
        hovered: Box::new(move |f, t| {
            let s = make(false)(f, t);
            if selected { s } else { row_style_hover(t, s) }
        }),
        pressed: Box::new(make(true)),
        disabled: Box::new(move |t| make(false)(false, t)),
    }
}

fn row_style_hover(t: &cosmic::Theme, mut s: cosmic::widget::button::Style) -> cosmic::widget::button::Style {
    let on: Color = t.cosmic().on_bg_color().into();
    s.background = Some(Background::Color(Color { a: 0.07, ..on }));
    s
}

pub fn view(app: &SettingsApp) -> Element<'_, Message> {
    // The page to highlight: the active page, or the nearest listed ancestor.
    let listed = |id: &str| GROUPS.iter().any(|g| g.iter().any(|(pid, _, _)| *pid == id));
    let mut highlight = Some(app.active_page);
    while let Some(p) = highlight {
        if listed(&app.pages.info[p].id) {
            break;
        }
        highlight = app.pages.info[p].parent;
    }

    let search = text_input::search_input("Search", &app.search_input)
        .id(app.search_id.clone())
        .on_input(Message::SearchActivateWith)
        .on_clear(Message::SearchClear)
        .on_submit(|_| Message::SearchSubmit)
        .width(Length::Fill);

    let mut list = column::with_capacity(64).spacing(1);
    for (gi, group) in GROUPS.iter().enumerate() {
        let mut any = false;
        for (id, label, hex) in group.iter() {
            let Some((entity, info)) = app.pages.info.iter().find(|(_, i)| i.id == *id) else { continue };
            if gi > 0 && !any {
                list = list.push(container(cosmic::widget::Space::new().height(Length::Fixed(12.0))));
            }
            any = true;
            let title = if label.is_empty() { info.title.clone() } else { label.to_string() };
            let selected = highlight == Some(entity);
            list = list.push(
                button::custom(
                    row![squircle(&info.icon_name, *hex), text::body(title)]
                        .spacing(9)
                        .align_y(Alignment::Center),
                )
                .width(Length::Fill)
                .padding([4, 8])
                .class(row_style(selected, false))
                .on_press(Message::Page(entity)),
            );
        }
    }

    // The sidebar sits on its own slightly lighter rounded panel, like macOS.
    container(
        container(
            column![search, scrollable(list).height(Length::Fill)]
                .spacing(12)
                .padding([10, 10, 10, 10]),
        )
        .height(Length::Fill)
        .class(theme::Container::custom(|t| {
            let c = t.cosmic();
            let on: Color = c.on_bg_color().into();
            container::Style {
                background: Some(Background::Color(Color { a: if c.is_dark { 0.06 } else { 0.04 }, ..on })),
                border: Border { radius: 14.0.into(), width: 1.0, color: Color { a: 0.08, ..on } },
                ..Default::default()
            }
        })),
    )
    .padding([0, 0, 8, 8])
    .width(Length::Fixed(258.0))
    .height(Length::Fill)
    .into()
}
