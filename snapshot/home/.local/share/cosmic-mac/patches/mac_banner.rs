//! macOS notification banner for cosmic-notifications (cosmic-mac patches/banner.py).
//!
//! A rounded, softly frosted card with the app icon on the left, the app name small and
//! grey, a bold title and the message below, a close button, and a soft drop shadow so it
//! stands out from whatever is behind it. Everything scales with the display: 1.0 up to
//! 1080p, about 1.17 on a 2560-wide display and 1.5 on a 3840-wide (4K) one, read from
//! `cosmic-randr` for the output the banner appears on.

use std::sync::atomic::{AtomicU32, Ordering};

use cosmic::{
    Element,
    iced::{Alignment, Background, Border, Color, Length, Shadow, Vector},
    theme,
    widget::{self, button, container, icon, text},
};
use cosmic::iced::widget::{column, row};

static FACTOR: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0f32

pub fn factor() -> f32 {
    f32::from_bits(FACTOR.load(Ordering::Relaxed))
}

/// Room around the card for its shadow (the popup surface is transparent there).
pub fn pad() -> f32 {
    (22.0 * factor()).round()
}

/// Width of the whole popup surface: card plus shadow room.
pub fn width() -> f32 {
    (344.0 * factor()).round() + 2.0 * pad()
}

/// Re-read the display size. `output` is the output name when known, else the widest one.
pub fn refresh(output: Option<&str>) {
    let Ok(out) = std::process::Command::new("cosmic-randr").args(["list", "--kdl"]).output() else {
        return;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut name = String::new();
    let mut enabled = false;
    let mut scale = 1.0f32;
    let mut widest = 0.0f32;
    let mut wanted = None;
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("output \"") {
            name = rest.split('"').next().unwrap_or("").to_string();
            enabled = l.contains("enabled=#true");
            scale = 1.0;
        } else if enabled && l.starts_with("scale ") {
            scale = l[6..].trim().parse().unwrap_or(1.0);
        } else if enabled && l.starts_with("mode ") && l.contains("current=#true") {
            let mut it = l.split_whitespace().skip(1);
            let w: f32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
            let logical = w / scale.max(0.5);
            widest = widest.max(logical);
            if output == Some(name.as_str()) {
                wanted = Some(logical);
            }
        }
    }
    let w = wanted.unwrap_or(widest);
    if w > 0.0 {
        let f = (1.0 + (w - 1920.0) / 1920.0 * 0.5).clamp(1.0, 1.75);
        FACTOR.store(f.to_bits(), Ordering::Relaxed);
    }
}

fn dim(t: &cosmic::Theme) -> cosmic::iced::widget::text::Style {
    let mut c: Color = t.cosmic().on_bg_color().into();
    c.a = 0.6;
    cosmic::iced::widget::text::Style { color: Some(c), ..Default::default() }
}

/// Grey close button, like macOS (not the accent color).
fn close_style(t: &cosmic::Theme, hover: f32) -> button::Style {
    let on: Color = t.cosmic().on_bg_color().into();
    let grey = Color { a: 0.55, ..on };
    button::Style {
        background: (hover > 0.0).then(|| Background::Color(Color { a: hover, ..on })),
        border_radius: 999.0.into(),
        icon_color: Some(grey),
        text_color: Some(grey),
        ..button::Style::new()
    }
}

/// One banner. `body` is the already-built message text; `size()` of text is set here.
pub fn card<'a, M: Clone + 'static>(
    icon_el: Option<icon::Icon>,
    app_name: String,
    summary: String,
    body: Element<'a, M>,
    on_close: M,
    on_press: M,
) -> Element<'a, M> {
    let f = factor();
    let icon_size = (42.0 * f).round() as u16;
    let lead: Element<'a, M> = match icon_el {
        Some(i) => container(i.size(icon_size))
            .width(Length::Fixed(icon_size as f32))
            .height(Length::Fixed(icon_size as f32))
            .into(),
        None => widget::Space::new().width(Length::Fixed(icon_size as f32)).into(),
    };
    let close = button::custom(icon::from_name("window-close-symbolic").size((14.0 * f).round() as u16).symbolic(true))
        .padding((4.0 * f).round() as u16)
        .class(theme::Button::Custom {
            active: Box::new(|_, t| close_style(t, 0.0)),
            hovered: Box::new(|_, t| close_style(t, 0.10)),
            pressed: Box::new(|_, t| close_style(t, 0.18)),
            disabled: Box::new(|t| close_style(t, 0.0)),
        })
        .on_press(on_close);
    let top = row![
        text::caption(app_name)
            .size(11.0 * f)
            .class(theme::Text::Custom(dim))
            .width(Length::Fill)
            .wrapping(cosmic::iced::widget::text::Wrapping::None),
        close,
    ]
    .align_y(Alignment::Center);
    let text_col = column![
        top,
        text::body(summary)
            .size(14.0 * f)
            .font(cosmic::font::bold())
            .width(Length::Fill),
        body,
    ]
    .spacing((2.0 * f).round())
    .width(Length::Fill);

    let card = container(row![lead, text_col].spacing((12.0 * f).round()).align_y(Alignment::Start))
        .padding([(12.0 * f).round() as u16, (14.0 * f).round() as u16])
        .width(Length::Fixed((344.0 * f).round()))
        .class(theme::Container::custom(move |t| {
            let c = t.cosmic();
            let on: Color = c.on_bg_color().into();
            // same frosted tint as the System Settings sidebar
            let tint = if c.is_dark { Color::from_rgba(0.15, 0.15, 0.16, 0.96) } else { Color::from_rgba(0.925, 0.925, 0.945, 0.96) };
            container::Style {
                background: Some(Background::Color(tint)),
                text_color: Some(on),
                icon_color: Some(on),
                border: Border { radius: (18.0 * f).into(), width: 1.0, color: Color { a: 0.10, ..on } },
                shadow: Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, if c.is_dark { 0.55 } else { 0.30 }),
                    offset: Vector::new(0.0, 8.0 * f),
                    blur_radius: 26.0 * f,
                },
                snap: true,
            }
        }));
    let p = pad();
    widget::mouse_area(container(card).padding([p, p, p, p])).on_press(on_press).into()
}
