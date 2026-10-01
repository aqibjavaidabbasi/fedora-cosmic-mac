//! macOS System Settings sidebar for COSMIC Settings (cosmic-mac patches/settings.py).
//!
//! The window is drawn like macOS: traffic lights and a see-through, blurred sidebar
//! (vibrancy) with a flat list of pages (search on top, colored rounded icons, blue
//! selection) beside an opaque content area. COSMIC's header bar is hidden. Pages not listed here
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
    container(icon::from_name(glyph.to_string()).size(17).symbolic(true))
        .width(Length::Fixed(28.0))
        .height(Length::Fixed(28.0))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .class(theme::Container::custom(move |_| container::Style {
            background: Some(Background::Color(bg)),
            border: Border { radius: 7.0.into(), ..Default::default() },
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
                (Some(Background::Color(Color { a: if c.is_dark { 0.16 } else { 0.10 }, ..on })), on)
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
                    row![squircle(&info.icon_name, *hex), text::body(title).size(14)]
                        .spacing(11)
                        .align_y(Alignment::Center),
                )
                .width(Length::Fill)
                .padding([5, 8])
                .class(row_style(selected, false))
                .on_press(Message::Page(entity)),
            );
        }
    }

    // Traffic lights (close, minimize, zoom) on a draggable strip, like macOS.
    let light = |hex: u32, msg: Message| {
        button::custom(cosmic::widget::Space::new().width(Length::Fixed(14.0)).height(Length::Fixed(14.0)))
            .padding(0)
            .width(Length::Fixed(14.0))
            .height(Length::Fixed(14.0))
            .class(theme::Button::Custom {
                active: Box::new(move |_, _| traffic(hex, 1.0)),
                hovered: Box::new(move |_, _| traffic(hex, 0.8)),
                pressed: Box::new(move |_, _| traffic(hex, 0.6)),
                disabled: Box::new(move |_| traffic(hex, 1.0)),
            })
            .on_press(msg)
    };
    let lights = cosmic::widget::mouse_area(
        container(
            row![light(0xFF5F57, Message::MacClose), light(0xFEBC2E, Message::MacMinimize), light(0x28C840, Message::MacMaximize)]
                .spacing(8),
        )
        .width(Length::Fill)
        .padding([14, 16, 10, 16]),
    )
    .on_press(Message::MacDrag)
    .on_double_click(Message::MacMaximize);

    column![lights, container(column![search, scrollable(list).height(Length::Fill)].spacing(12)).padding([0, 10, 10, 10])]
        .width(Length::Fixed(250.0))
        .height(Length::Fill)
        .into()
}

fn traffic(hex: u32, dim: f32) -> cosmic::widget::button::Style {
    let c = rgb(hex);
    cosmic::widget::button::Style {
        background: Some(Background::Color(Color { r: c.r * dim, g: c.g * dim, b: c.b * dim, a: 1.0 })),
        border_radius: 7.0.into(),
        border_width: 0.5,
        border_color: Color::from_rgba(0.0, 0.0, 0.0, 0.2),
        ..cosmic::widget::button::Style::new()
    }
}

/// The whole macOS-style window: a see-through, blurred sidebar (vibrancy) beside an
/// opaque content area with a draggable toolbar strip on top.
pub fn window<'a>(app: &'a SettingsApp, content: Element<'a, Message>) -> Element<'a, Message> {
    let toolbar = cosmic::widget::mouse_area(container(cosmic::widget::Space::new().height(Length::Fixed(10.0))).width(Length::Fill))
        .on_press(Message::MacDrag)
        .on_double_click(Message::MacMaximize);
    let content = container(column![toolbar, content].height(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill)
        .class(theme::Container::custom(|t| {
            let c = t.cosmic();
            container::Style {
                background: Some(Background::Color(c.background(false).base.into())),
                border: Border { radius: [0.0, 12.0, 12.0, 0.0].into(), ..Default::default() },
                ..Default::default()
            }
        }));
    let sidebar = container(view(app)).height(Length::Fill).class(theme::Container::custom(|t| {
        let c = t.cosmic();
        // Frosted look without relying on compositor blur (COSMIC doesn't blur app
        // windows yet, and an unblurred see-through sidebar shows text behind it):
        // a nearly opaque, slightly cool tint that lets only a hint of colour through.
        let tint = if c.is_dark { Color::from_rgba(0.15, 0.15, 0.16, 0.94) } else { Color::from_rgba(0.925, 0.925, 0.945, 0.94) };
        let on: Color = c.on_bg_color().into();
        container::Style {
            background: Some(Background::Color(tint)),
            border: Border { radius: [12.0, 0.0, 0.0, 12.0].into(), width: 0.0, color: Color { a: 0.0, ..on } },
            ..Default::default()
        }
    }));
    let divider = container(cosmic::widget::Space::new().width(Length::Fixed(1.0)))
        .height(Length::Fill)
        .class(theme::Container::custom(|t| {
            let on: Color = t.cosmic().on_bg_color().into();
            container::Style { background: Some(Background::Color(Color { a: 0.08, ..on })), ..Default::default() }
        }));
    row![sidebar, divider, content].height(Length::Fill).into()
}
