//! macOS-style surfaces: 16px cards with a hairline border, pills and round badges.
//! Colors derive from the active COSMIC theme so light/dark both work.

use cosmic::{
    Theme,
    iced::core::{Background, Border, Color, Shadow},
    iced::widget::container,
    theme,
};

pub const CARD_RADIUS: f32 = 16.0;
pub const LOW_POWER_YELLOW: Color = Color::from_rgb(1.0, 0.8, 0.0); // #FFCC00

fn hairline(t: &Theme) -> Color {
    if t.cosmic().is_dark {
        Color::from_rgba(1.0, 1.0, 1.0, 0.10)
    } else {
        Color::from_rgba(0.0, 0.0, 0.0, 0.08)
    }
}

fn plain(background: Color, radius: f32, border: Color) -> container::Style {
    container::Style {
        icon_color: None,
        text_color: None,
        background: Some(Background::Color(background)),
        border: Border { radius: radius.into(), width: 1.0, color: border },
        shadow: Shadow::default(),
        snap: true,
    }
}

/// Frosted "tile" inside the popover (Low Power Mode card, etc.).
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

/// Circular icon badge; accent-filled when `active`, like macOS' blue toggles.
pub fn badge(active: bool) -> theme::Container<'static> {
    theme::Container::custom(move |t| {
        let (fill, fg) = if active {
            (t.cosmic().accent_color().into(), Color::WHITE)
        } else if t.cosmic().is_dark {
            (Color::from_rgba(1.0, 1.0, 1.0, 0.14), Color::WHITE)
        } else {
            (Color::from_rgba(0.0, 0.0, 0.0, 0.08), Color::BLACK)
        };
        container::Style {
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
        container::Style {
            icon_color: fg,
            text_color: fg,
            ..plain(Color::TRANSPARENT, 0.0, Color::TRANSPARENT)
        }
    })
}
