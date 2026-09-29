//! macOS-style surfaces: popover sheet, 16px cards with a hairline border, pills and
//! round badges. Colors derive from the active COSMIC theme so light/dark both work.
//! Shared by the cosmic-mac applets (the same file is copied into each crate).

#![allow(dead_code)]

use std::sync::LazyLock;

use cosmic::{
    Element, Theme,
    iced::core::{Background, Border, Color, Shadow},
    iced::widget::container as iced_container,
    iced::{Alignment, Length, Limits},
    theme,
    widget::{autosize, container, divider, text},
};

pub const CARD_RADIUS: f32 = 16.0;
pub const POPOVER_RADIUS: f32 = 16.0;
pub const LOW_POWER_YELLOW: Color = Color::from_rgb(1.0, 0.8, 0.0); // #FFCC00
pub const MAC_BLUE: Color = Color::from_rgb(0.0, 0.478, 1.0); // #007AFF

static POPOVER_ID: LazyLock<cosmic::widget::Id> = LazyLock::new(|| cosmic::widget::Id::new("cosmic-mac-popover"));

fn hairline(t: &Theme) -> Color {
    if t.cosmic().is_dark {
        Color::from_rgba(1.0, 1.0, 1.0, 0.10)
    } else {
        Color::from_rgba(0.0, 0.0, 0.0, 0.08)
    }
}

fn plain(background: Color, radius: f32, border: Color) -> iced_container::Style {
    iced_container::Style {
        icon_color: None,
        text_color: None,
        background: Some(Background::Color(background)),
        border: Border { radius: radius.into(), width: 1.0, color: border },
        shadow: Shadow::default(),
        snap: true,
    }
}

/// Secondary (grey) text color, like macOS' secondaryLabelColor.
pub fn dim(t: &Theme) -> Color {
    let mut c: Color = t.cosmic().background(t.transparent).on.into();
    c.a = 0.55;
    c
}

fn dim_text(t: &Theme) -> cosmic::iced::widget::text::Style {
    cosmic::iced::widget::text::Style { color: Some(dim(t)), ..Default::default() }
}

/// Grey text such as "Power Source: Battery".
pub fn secondary<'a, M: 'a>(s: impl Into<String>) -> Element<'a, M> {
    text::body(s.into()).class(theme::Text::Custom(dim_text)).into()
}

/// Grey bold section header such as "Energy Mode" or "Known Networks".
pub fn section<'a, M: 'a>(s: impl Into<String>) -> Element<'a, M> {
    text::heading(s.into()).class(theme::Text::Custom(dim_text)).into()
}

/// The macOS menu-bar popover sheet: fixed width, rounded, frosted theme background.
pub fn popover<'a, M: 'static>(content: impl Into<Element<'a, M>>, width: f32) -> Element<'a, M> {
    let sheet = container(content).width(Length::Fill).class(theme::Container::custom(|t| {
        let bg = t.cosmic().background(t.transparent);
        iced_container::Style {
            text_color: Some(bg.on.into()),
            icon_color: Some(bg.on.into()),
            background: Some(Background::Color(bg.base.into())),
            border: Border { radius: POPOVER_RADIUS.into(), width: 1.0, color: hairline(t) },
            shadow: Shadow::default(),
            snap: true,
        }
    }));
    autosize::autosize(
        container(sheet).height(Length::Shrink).align_x(Alignment::Center).align_y(Alignment::Start),
        POPOVER_ID.clone(),
    )
    .limits(Limits::NONE.min_height(1.).min_width(width).max_width(width).max_height(1200.0))
    .into()
}

/// Frosted "tile" inside the popover.
pub fn card() -> theme::Container<'static> {
    theme::Container::custom(|t| {
        let fill = if t.cosmic().is_dark {
            Color::from_rgba(1.0, 1.0, 1.0, 0.06)
        } else {
            Color::from_rgba(1.0, 1.0, 1.0, 0.55)
        };
        plain(fill, CARD_RADIUS, hairline(t))
    })
}

/// Circular icon badge; Mac blue when `active`, grey otherwise.
pub fn badge(active: bool) -> theme::Container<'static> {
    theme::Container::custom(move |t| {
        let (fill, fg) = if active {
            (MAC_BLUE, Color::WHITE)
        } else if t.cosmic().is_dark {
            (Color::from_rgba(1.0, 1.0, 1.0, 0.14), Color::WHITE)
        } else {
            (Color::from_rgba(0.0, 0.0, 0.0, 0.08), Color::from_rgba(0.0, 0.0, 0.0, 0.75))
        };
        iced_container::Style {
            icon_color: Some(fg),
            text_color: Some(fg),
            ..plain(fill, 999.0, Color::TRANSPARENT)
        }
    })
}

/// Tints the panel glyph + percentage yellow while Low Power Mode is on.
pub fn low_power_tint(on: bool) -> theme::Container<'static> {
    theme::Container::custom(move |_| {
        let fg = on.then_some(LOW_POWER_YELLOW);
        iced_container::Style {
            icon_color: fg,
            text_color: fg,
            ..plain(Color::TRANSPARENT, 0.0, Color::TRANSPARENT)
        }
    })
}

/// A macOS menu separator, inset to the text column.
pub fn separator<'a, M: 'a>() -> Element<'a, M> {
    container(divider::horizontal::light()).padding([5, 14]).width(Length::Fill).into()
}

/// True once when `$XDG_RUNTIME_DIR/cosmic-mac/open/<app id>` exists (the file is removed).
/// Lets scripts and keyboard shortcuts open a popover:
///   mkdir -p $XDG_RUNTIME_DIR/cosmic-mac/open && touch $XDG_RUNTIME_DIR/cosmic-mac/open/<app id>
pub fn open_requested(app_id: &str) -> bool {
    let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") else { return false };
    std::fs::remove_file(format!("{dir}/cosmic-mac/open/{app_id}")).is_ok()
}

/// Small grey caption (11px), e.g. the subtitle under a Control Center tile.
pub fn secondary_small<'a, M: 'a>(s: impl Into<String>) -> Element<'a, M> {
    text::caption(s.into())
        .size(11)
        .wrapping(cosmic::iced::core::widget::text::Wrapping::None)
        .class(theme::Text::Custom(dim_text))
        .into()
}
