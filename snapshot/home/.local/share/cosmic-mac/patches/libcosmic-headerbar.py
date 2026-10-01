#!/usr/bin/env python3
"""macOS title bars for COSMIC apps: patch libcosmic's header bar in Cargo's git checkouts.

Usage: libcosmic-headerbar.py   (patches every ~/.cargo/git/checkouts/libcosmic-*/<rev>)

COSMIC's own apps (Terminal, Text Editor, Media Player...) draw their title bar with
libcosmic's header_bar. This moves the window buttons to the start of the bar as macOS
traffic lights - close, minimize, zoom; 14px circles, a little larger than macOS so they
are easy to hit; grey while the window is in the background. Idempotent. Apps must be
rebuilt afterwards (rebuild.sh does `cargo clean -p libcosmic` for them).
Exits 1 if a checkout no longer matches.
"""
import glob, os, re, sys

MARK = "// cosmic-mac: macOS traffic lights"

CONTROLS = r'''    /// Creates the widget for window controls.
    fn window_controls(&mut self, spacing: u16) -> Element<'a, Message> {
        // cosmic-mac: macOS traffic lights (close, minimize, zoom) - see window_controls_mac
        let _ = spacing;
        self.window_controls_mac()
    }

    fn window_controls_mac(&mut self) -> Element<'a, Message> {
        let focused = self.focused;
        let light = |hex: u32, msg: Option<Message>| -> Element<'a, Message> {
            let base = iced::Color::from_rgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
            let style = move |dim: f32| widget::button::Style {
                background: Some(iced::Background::Color(if focused {
                    iced::Color { r: base.r * dim, g: base.g * dim, b: base.b * dim, a: 1.0 }
                } else {
                    iced::Color::from_rgba(0.5, 0.5, 0.5, 0.35)
                })),
                border_radius: 7.0.into(),
                border_width: 0.5,
                border_color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.18),
                ..widget::button::Style::new()
            };
            widget::button::custom(widget::Space::new().width(Length::Fixed(14.0)).height(Length::Fixed(14.0)))
                .padding(0)
                .class(theme::Button::Custom {
                    active: Box::new(move |_, _| style(1.0)),
                    hovered: Box::new(move |_, _| style(0.88)),
                    pressed: Box::new(move |_, _| style(0.72)),
                    disabled: Box::new(move |_| style(1.0)),
                })
                .on_press_maybe(msg)
                .into()
        };
        let close = self.on_close.take();
        let minimize = self.on_minimize.take();
        let maximize = self.on_maximize.take();
        if close.is_none() && minimize.is_none() && maximize.is_none() {
            return widget::Space::new().width(Length::Fixed(0.0)).into();
        }
        widget::row::with_capacity(3)
            .push(light(0xFF5F57, close))
            .push(light(0xFEBC2E, minimize))
            .push(light(0x28C840, maximize))
            .spacing(8)
            .padding([0, 6])
            .align_y(iced::Alignment::Center)
            .into()
    }
}

impl<'a, Message: Clone + 'static> From<HeaderBar<'a, Message>> for Element<'a, Message> {'''


def patch(path):
    s = open(path).read()
    if MARK in s:
        return True
    s, n1 = re.subn(r"let start = std::mem::take\(&mut self\.start\);", "let mut start = std::mem::take(&mut self.start);", s, count=1)
    s, n2 = re.subn(
        r"        // Also packs the window controls at the very end\.\n        end\.push\(self\.window_controls\(space_xxs\)\);\n",
        "        " + MARK + " at the very start\n        start.insert(0, self.window_controls(space_xxs));\n        let _ = &mut end;\n",
        s, count=1)
    s, n3 = re.subn(
        r"    /// Creates the widget for window controls\.\n    fn window_controls\(&mut self, spacing: u16\) -> Element<'a, Message> \{\n.*?\n\}\n\nimpl<'a, Message: Clone \+ 'static> From<HeaderBar<'a, Message>> for Element<'a, Message> \{",
        lambda m: CONTROLS, s, count=1, flags=re.S)
    if (n1, n2, n3) != (1, 1, 1):
        print(f"libcosmic header bar patch failed for {path}: {(n1, n2, n3)}", file=sys.stderr)
        return False
    open(path, "w").write(s)
    return True


ok = True
for path in glob.glob(os.path.expanduser("~/.cargo/git/checkouts/libcosmic-*/*/src/widget/header_bar.rs")):
    ok = patch(path) and ok
sys.exit(0 if ok else 1)
