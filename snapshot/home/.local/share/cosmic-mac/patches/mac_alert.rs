//! macOS-style alert for COSMIC's system dialogs (cosmic-mac patches/osd.py):
//! a small frosted card with the icon on top, a bold centered title, a short
//! centered message, then capsule buttons - blue for the default action.
//! Two buttons sit side by side; three are stacked, like macOS.

use cosmic::{
    Element,
    iced::{Alignment, Background, Border, Color, Length, Shadow, Vector},
    theme,
    widget::{self, button, container, text},
};

const MAC_BLUE: Color = Color::from_rgb(0.039, 0.518, 1.0); // #0A84FF

/// A button: label, message (None = disabled), keyboard-focus id.
pub struct Action<M> {
    pub label: String,
    pub on_press: Option<M>,
    pub id: Option<widget::Id>,
}

impl<M> Action<M> {
    pub fn new(label: impl Into<String>, on_press: Option<M>) -> Self {
        Self { label: label.into(), on_press, id: None }
    }
    pub fn id(mut self, id: widget::Id) -> Self {
        self.id = Some(id);
        self
    }
}

fn capsule(primary: bool) -> theme::Button {
    let style = move |hover: f32, t: &cosmic::Theme| {
        let c = t.cosmic();
        let on: Color = c.on_bg_color().into();
        let (bg, fg) = if primary {
            (Color { a: 1.0 - hover, ..MAC_BLUE }, Color::WHITE)
        } else {
            (Color { a: if c.is_dark { 0.16 } else { 0.08 } + hover, ..on }, on)
        };
        button::Style {
            background: Some(Background::Color(bg)),
            border_radius: 999.0.into(),
            text_color: Some(fg),
            icon_color: Some(fg),
            ..button::Style::new()
        }
    };
    theme::Button::Custom {
        active: Box::new(move |_, t| style(0.0, t)),
        hovered: Box::new(move |_, t| style(0.08, t)),
        pressed: Box::new(move |_, t| style(0.16, t)),
        disabled: Box::new(move |t| {
            let mut s = style(0.0, t);
            s.background = s.background.map(|b| match b {
                Background::Color(c) => Background::Color(Color { a: c.a * 0.5, ..c }),
                other => other,
            });
            s
        }),
    }
}

fn mac_button<'a, M: Clone + 'static>(a: Action<M>, primary: bool) -> Element<'a, M> {
    let mut b = button::custom(
        text::body(a.label)
            .width(Length::Fill)
            .align_x(cosmic::iced::widget::text::Alignment::Center),
    )
    .width(Length::Fill)
    .padding([6, 12])
    .class(capsule(primary))
    .on_press_maybe(a.on_press);
    if let Some(id) = a.id {
        b = b.id(id);
    }
    b.into()
}

/// The whole alert card. `default` is the blue button; `others` go before it
/// (Cancel, then e.g. Restart when there are three).
pub fn alert<'a, M: Clone + 'static>(
    icon: Element<'a, M>,
    title: impl Into<String>,
    body: impl Into<String>,
    control: Option<Element<'a, M>>,
    cancel: Action<M>,
    default: Action<M>,
    extra: Option<Action<M>>,
) -> Element<'a, M> {
    let mut col = widget::column::with_capacity(6)
        .push(icon)
        .push(
            text::heading(title.into())
                .size(15)
                .width(Length::Fill)
                .align_x(cosmic::iced::widget::text::Alignment::Center),
        )
        .spacing(10)
        .align_x(Alignment::Center);
    let body = body.into();
    if !body.is_empty() {
        col = col.push(
            text::caption(body)
                .size(12)
                .width(Length::Fill)
                .align_x(cosmic::iced::widget::text::Alignment::Center),
        );
    }
    if let Some(c) = control {
        col = col.push(container(c).padding([4, 0]).width(Length::Fill));
    }
    let buttons: Element<'a, M> = match extra {
        None => widget::row::with_children(vec![mac_button(cancel, false), mac_button(default, true)])
            .spacing(8)
            .into(),
        Some(extra) => widget::column::with_children(vec![
            mac_button(default, true),
            mac_button(extra, false),
            mac_button(cancel, false),
        ])
        .spacing(8)
        .into(),
    };
    col = col.push(container(buttons).padding([6, 0, 0, 0]).width(Length::Fill));

    let card = container(col)
        .width(Length::Fixed(280.0))
        .padding([22, 18, 16, 18])
        .class(theme::Container::custom(|t| {
            let c = t.cosmic();
            let mut bg: Color = c.background(false).base.into();
            bg.a = 0.94;
            let on: Color = c.on_bg_color().into();
            container::Style {
                background: Some(Background::Color(bg)),
                text_color: Some(on),
                icon_color: Some(on),
                border: Border { radius: 18.0.into(), width: 1.0, color: Color { a: 0.10, ..on } },
                shadow: Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.35), offset: Vector::new(0.0, 8.0), blur_radius: 24.0 },
                snap: true,
            }
        }));
    // room around the card for its shadow
    container(card).padding(28).into()
}
