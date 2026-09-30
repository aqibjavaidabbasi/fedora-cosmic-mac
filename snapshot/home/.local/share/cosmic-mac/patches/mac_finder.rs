//! macOS Finder look for COSMIC Files (cosmic-mac patches/files.py).
//!
//! No COSMIC header bar: traffic lights sit on top of a see-through sidebar, and the
//! content area gets a Finder toolbar - back/forward, the folder name in bold, then the
//! grid/list switch, search and the File/Edit/View/Sort menus on the right.

use cosmic::{
    Element,
    iced::{Alignment, Background, Border, Color, Length},
    theme,
    widget::{self, button, container, icon, row, text},
};

use super::{App, Message};
use crate::{menu, tab};

type Act = cosmic::Action<Message>;

fn rgb(hex: u32) -> Color {
    Color::from_rgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn traffic(hex: u32, dim: f32) -> button::Style {
    let c = rgb(hex);
    button::Style {
        background: Some(Background::Color(Color { r: c.r * dim, g: c.g * dim, b: c.b * dim, a: 1.0 })),
        border_radius: 6.0.into(),
        border_width: 0.5,
        border_color: Color::from_rgba(0.0, 0.0, 0.0, 0.2),
        ..button::Style::new()
    }
}

/// Close / minimize / zoom on a draggable strip.
fn lights<'a>() -> Element<'a, Act> {
    let light = |hex: u32, msg: Message| {
        button::custom(widget::Space::new().width(Length::Fixed(12.0)).height(Length::Fixed(12.0)))
            .padding(0)
            .class(theme::Button::Custom {
                active: Box::new(move |_, _| traffic(hex, 1.0)),
                hovered: Box::new(move |_, _| traffic(hex, 0.8)),
                pressed: Box::new(move |_, _| traffic(hex, 0.6)),
                disabled: Box::new(move |_| traffic(hex, 1.0)),
            })
            .on_press(cosmic::Action::App(msg))
    };
    widget::mouse_area(
        container(
            row![light(0xFF5F57, Message::WindowClose), light(0xFEBC2E, Message::MacMinimize), light(0x28C840, Message::MacMaximize)]
                .spacing(8),
        )
        .width(Length::Fill)
        .padding([16, 18, 12, 18]),
    )
    .on_press(cosmic::Action::App(Message::MacDrag))
    .on_double_click(cosmic::Action::App(Message::MacMaximize))
    .into()
}

/// The sidebar: traffic lights above COSMIC's places list, on a frosted tint.
pub fn sidebar<'a>(nav: Element<'a, Act>) -> Element<'a, Act> {
    container(widget::column::with_children(vec![lights(), nav]))
        .height(Length::Fill)
        .class(theme::Container::custom(|t| {
            let c = t.cosmic();
            // COSMIC doesn't blur app windows yet, so a nearly opaque cool tint stands in
            // for the vibrancy (same as the System Settings sidebar).
            let tint = if c.is_dark { Color::from_rgba(0.15, 0.15, 0.16, 0.94) } else { Color::from_rgba(0.925, 0.925, 0.945, 0.94) };
            container::Style {
                background: Some(Background::Color(tint)),
                border: Border { radius: [12.0, 0.0, 0.0, 12.0].into(), ..Default::default() },
                ..Default::default()
            }
        }))
        .into()
}

fn icon_button<'a>(name: &'static str, msg: Option<Message>) -> Element<'a, Message> {
    button::custom(icon::from_name(name).size(18))
        .padding([6, 8])
        .class(theme::Button::Icon)
        .on_press_maybe(msg)
        .into()
}

/// A two-part pill switching between icon grid and list, like Finder's view control.
fn view_switch<'a>(current: Option<tab::View>) -> Element<'a, Message> {
    let part = |name: &'static str, view: tab::View| {
        let on = current == Some(view);
        button::custom(icon::from_name(name).size(16))
            .padding([5, 10])
            .class(if on { theme::Button::Standard } else { theme::Button::Icon })
            .on_press(Message::TabView(None, view))
    };
    container(row![part("view-grid-symbolic", tab::View::Grid), part("view-list-symbolic", tab::View::List)].spacing(2))
        .padding(2)
        .class(theme::Container::custom(|t| {
            let on: Color = t.cosmic().on_bg_color().into();
            container::Style {
                background: Some(Background::Color(Color { a: 0.06, ..on })),
                border: Border { radius: 8.0.into(), ..Default::default() },
                ..Default::default()
            }
        }))
        .into()
}

/// Finder toolbar above the files, then the files; on the window's opaque side.
pub fn content<'a>(app: &'a App, files: Element<'a, Message>) -> Element<'a, Message> {
    let tab = app.tab_model.active_data::<tab::Tab>();
    let back = tab.is_some_and(|t| t.history_i > 0 && !t.history.is_empty());
    let forward = tab.is_some_and(|t| t.history_i + 1 < t.history.len());
    let title = tab.map(|t| t.title()).unwrap_or_default();

    let search: Element<'a, Message> = match app.search_get() {
        Some(term) => widget::text_input::search_input("Search", term)
            .width(Length::Fixed(200.0))
            .id(app.search_id.clone())
            .on_clear(Message::SearchClear)
            .on_input(Message::SearchInput)
            .into(),
        None => icon_button("system-search-symbolic", Some(Message::SearchActivate)),
    };

    let bar = row![
        icon_button("go-previous-symbolic", back.then_some(Message::TabMessage(None, tab::Message::GoPrevious))),
        icon_button("go-next-symbolic", forward.then_some(Message::TabMessage(None, tab::Message::GoNext))),
        text::title4(title).wrapping(cosmic::iced::widget::text::Wrapping::None),
        widget::space::horizontal().width(Length::Fill),
        view_switch(tab.map(|t| t.config.view.clone())),
        search,
        menu::menu_bar(&app.core, tab, &app.config, &app.modifiers, &app.key_binds, app.clipboard_has_content()),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let toolbar = widget::mouse_area(container(bar).padding([8, 12, 6, 8]).width(Length::Fill))
        .on_press(Message::MacDrag)
        .on_double_click(Message::MacMaximize);

    let divider = container(widget::Space::new().height(Length::Fixed(1.0)))
        .width(Length::Fill)
        .class(theme::Container::custom(|t| {
            let on: Color = t.cosmic().on_bg_color().into();
            container::Style { background: Some(Background::Color(Color { a: 0.08, ..on })), ..Default::default() }
        }));

    container(widget::column::with_children(vec![toolbar.into(), divider.into(), files]))
        .width(Length::Fill)
        .height(Length::Fill)
        .class(theme::Container::custom(|t| {
            let c = t.cosmic();
            container::Style {
                background: Some(Background::Color(c.background(false).base.into())),
                border: Border { radius: [0.0, 12.0, 12.0, 0.0].into(), ..Default::default() },
                ..Default::default()
            }
        }))
        .into()
}
