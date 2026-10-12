#!/usr/bin/env python3
"""Patch cosmic-launcher into a macOS Tahoe Spotlight (run from its source root).
Exits 1 if the code no longer matches.

- A "Spotlight Search" pill centered about a third of the way down the screen,
  with four round buttons beside it (Apps, Files, Shortcuts, Settings).
- Nothing below the pill until you type; then a rounded results panel with
  app icon, bold name, grey subtitle and a highlighted selection.
- The window switcher (Alt+Tab) looks like macOS Command-Tab: a row of big app
  icons in a rounded panel, the selected one highlighted with its name below.
"""
import re, sys

p = "src/app.rs"
s = open(p).read()

def sub(pat, rep, flags=0):
    global s
    s, n = re.subn(pat, rep, s, count=1, flags=flags)
    if n != 1:
        print(f"launcher patch failed: {pat[:70]}", file=sys.stderr)
        sys.exit(1)

# wider surface for the pill + round buttons
sub(r"(size_limits: Limits::NONE\.min_width\(1\.0\)\.min_height\(1\.0\)\.max_width\()600\.0(\),\n\s*exclusive_zone: -1,)",
    r"\g<1>1600.0\2")

# new message for the round buttons
sub(r"(pub enum Message \{\n    InputChanged\(String\),)", r"\1\n    MacOpen(&'static str),")
sub(r"(    fn update\(&mut self, message: Message\) -> Task<Self::Message> \{\n        match message \{\n)",
    r"""\1            Message::MacOpen(what) => {
                let (cmd, args): (&str, &[&str]) = match what {
                    "apps" => ("cosmic-app-library", &[]),
                    "files" => ("nautilus", &["--new-window"]),
                    "shortcuts" => ("cosmic-settings", &["keyboard"]),
                    _ => ("cosmic-settings", &[]),
                };
                if let Ok(mut c) = std::process::Command::new(cmd).args(args).spawn() {
                    std::thread::spawn(move || {
                        let _ = c.wait();
                    });
                }
                return self.hide();
            }
""")

VIEW = r'''            // ---- macOS Spotlight (cosmic-mac patches/launcher.py) ----
            let pill_style = |theme: &cosmic::Theme| {
                let t = theme.cosmic();
                let mut a = theme.focused(&cosmic::theme::TextInput::Search);
                let mut bg: Color = t.background(theme.transparent).base.into();
                bg.a = bg.a.max(0.92);
                a.background = bg.into();
                a.border_radius = 26.0.into();
                a.border_width = 1.0;
                a.border_color = if t.is_dark { Color::from_rgba(1.0, 1.0, 1.0, 0.14) } else { Color::from_rgba(0.0, 0.0, 0.0, 0.10) };
                a.border_offset = None;
                a
            };
            let launcher_entry = text_input::search_input(String::from("Spotlight Search"), &self.input_value)
                .on_input(Message::InputChanged)
                .on_paste(Message::InputChanged)
                .on_submit(|_| Message::Activate(None))
                .on_tab(Message::TabPress)
                .size(20)
                .padding([12, 20])
                .style(cosmic::theme::TextInput::Custom {
                    active: Box::new(pill_style),
                    error: Box::new(pill_style),
                    hovered: Box::new(pill_style),
                    focused: Box::new(pill_style),
                    disabled: Box::new(pill_style),
                })
                .width(MAC_BAR_W)
                .id(INPUT_ID.clone())
                .always_active();

            let circle = |icon_name: &'static str, what: &'static str| {
                cosmic::widget::button::custom(
                    container(icon::from_name(icon_name).size(22).symbolic(true))
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .align_x(Horizontal::Center)
                        .align_y(Vertical::Center),
                )
                .width(Length::Fixed(52.0))
                .height(Length::Fixed(52.0))
                .padding(0)
                .on_press(Message::MacOpen(what))
                .class(Button::Custom {
                    active: Box::new(|_, theme| mac_circle(theme, 0.0)),
                    hovered: Box::new(|_, theme| mac_circle(theme, 0.08)),
                    pressed: Box::new(|_, theme| mac_circle(theme, 0.14)),
                    disabled: Box::new(|theme| mac_circle(theme, 0.0)),
                })
            };

            let rows: Vec<Element<'_, Message>> = self
                .launcher_items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let (name, desc) = if item.window.is_some() {
                        (&item.description, &item.name)
                    } else {
                        (&item.name, &item.description)
                    };
                    let is_focused = i == self.focused;
                    let title = text::body(name.lines().next().unwrap_or_default().to_string())
                        .font(cosmic::font::semibold())
                        .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)));
                    let subtitle = text::caption(desc.lines().next().unwrap_or_default().to_string())
                        .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                        .class(theme::Text::Custom(|t| {
                            let c = t.cosmic();
                            let mut color: Color = c.on_bg_color().into();
                            color.a = 0.6;
                            cosmic::iced::widget::text::Style { color: Some(color), selected_fill: c.accent_color().into(), ..Default::default() }
                        }));
                    let mut content = Vec::new();
                    match self.launcher_item_icon_handles.get(i) {
                        Some(Some(h)) => content.push(icon(h.clone()).width(Length::Fixed(32.0)).height(Length::Fixed(32.0)).into()),
                        _ => content.push(icon::from_name("application-x-executable").size(32).into()),
                    }
                    let mut text_col = column![title];
                    if !desc.trim().is_empty() {
                        text_col = text_col.push(subtitle);
                    }
                    content.push(text_col.width(Length::Fill).into());
                    mouse_area(
                        cosmic::widget::button::custom(row(content).spacing(10).align_y(Alignment::Center))
                            .id(self.result_ids[i].clone())
                            .width(Length::Fill)
                            .on_press(Message::Activate(Some(i)))
                            .padding([6, 10])
                            .class(Button::Custom {
                                active: Box::new(move |f, theme| mac_row(theme, is_focused || f, 0.0)),
                                hovered: Box::new(move |f, theme| mac_row(theme, is_focused || f, 0.06)),
                                pressed: Box::new(move |f, theme| mac_row(theme, is_focused || f, 0.1)),
                                disabled: Box::new(move |theme| mac_row(theme, false, 0.0)),
                            }),
                    )
                    .on_right_release(Message::Context(i))
                    .into()
                })
                .collect();

            // Suggestions only once something is typed (the switcher always lists windows).
            let show_rows = !rows.is_empty() && (self.alt_tab || !self.input_value.is_empty());
            let results = show_rows.then(|| {
                let list = Column::with_children(rows).spacing(2);
                let list: Element<'_, Message> = if self.launcher_items.len() > SCROLL_MIN {
                    container(scrollable(list).id(SCROLLABLE.clone())).max_height(460).into()
                } else {
                    list.into()
                };
                container(list)
                    .padding(8)
                    .width(Length::Fixed(if self.alt_tab { 600.0 } else { MAC_BAR_W }))
                    .class(Container::Custom(Box::new(|theme| {
                        let t = theme.cosmic();
                        let mut bg: Color = t.background(theme.transparent).base.into();
                        bg.a = bg.a.max(0.94);
                        container::Style {
                            text_color: Some(t.on_bg_color().into()),
                            icon_color: Some(t.on_bg_color().into()),
                            background: Some(bg.into()),
                            border: Border {
                                radius: 22.0.into(),
                                width: 1.0,
                                color: if t.is_dark { Color::from_rgba(1.0, 1.0, 1.0, 0.12) } else { Color::from_rgba(0.0, 0.0, 0.0, 0.08) },
                            },
                            shadow: Shadow::default(),
                            snap: true,
                        }
                    })))
            });

            let mut content = Column::new().spacing(10).align_x(Alignment::Start);
            if !self.alt_tab {
                content = content.push(
                    row![
                        launcher_entry,
                        circle("view-app-grid-symbolic", "apps"),
                        circle("folder-symbolic", "files"),
                        circle("input-keyboard-symbolic", "shortcuts"),
                        circle("emblem-system-symbolic", "settings"),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                );
            }
            if self.alt_tab {
                // macOS Command-Tab: one row of big icons, the selected one highlighted
                // with its name underneath.
                let mut strip = cosmic::widget::Row::new().spacing(6).align_y(Alignment::Start);
                for (i, item) in self.launcher_items.iter().enumerate() {
                    let selected = i == self.focused;
                    let glyph: Element<'_, Message> = match self.launcher_item_icon_handles.get(i) {
                        Some(Some(h)) => icon(h.clone()).width(Length::Fixed(72.0)).height(Length::Fixed(72.0)).into(),
                        _ => icon::from_name("application-x-executable").size(72).into(),
                    };
                    let app_name = if item.window.is_some() { &item.description } else { &item.name };
                    let label = text::caption(if selected { app_name.lines().next().unwrap_or_default().to_string() } else { String::new() })
                        .size(12)
                        .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)));
                    let tile = cosmic::widget::button::custom(
                        container(glyph).padding(6).class(Container::Custom(Box::new(move |_| container::Style {
                            background: selected.then(|| Color::from_rgba(1.0, 1.0, 1.0, 0.22).into()),
                            border: Border { radius: 16.0.into(), ..Default::default() },
                            ..Default::default()
                        }))),
                    )
                    .id(self.result_ids[i].clone())
                    .padding(0)
                    .on_press(Message::Activate(Some(i)))
                    .class(Button::Transparent);
                    strip = strip.push(
                        Column::new()
                            .push(tile)
                            .push(container(label).width(Length::Fixed(84.0)).center_x(Length::Fixed(84.0)).height(Length::Fixed(18.0)))
                            .align_x(Alignment::Center)
                            .width(Length::Fixed(84.0))
                            .spacing(4),
                    );
                }
                content = content.push(
                    container(strip)
                        .padding([14, 14, 8, 14])
                        .class(Container::Custom(Box::new(|theme| {
                            let t = theme.cosmic();
                            let mut bg: Color = t.background(theme.transparent).base.into();
                            bg.a = bg.a.max(0.9);
                            container::Style {
                                text_color: Some(t.on_bg_color().into()),
                                icon_color: Some(t.on_bg_color().into()),
                                background: Some(bg.into()),
                                border: Border {
                                    radius: 26.0.into(),
                                    width: 1.0,
                                    color: if t.is_dark { Color::from_rgba(1.0, 1.0, 1.0, 0.12) } else { Color::from_rgba(0.0, 0.0, 0.0, 0.08) },
                                },
                                shadow: Shadow::default(),
                                snap: true,
                            }
                        }))),
                );
            } else if let Some(r) = results {
                content = content.push(r);
            }

            let window = Column::new()
                .push(vertical_space().height(Length::Fixed(if self.alt_tab { *MAC_TOP + 60.0 } else { *MAC_TOP })))
                .push(id_container(content, MAIN_ID.clone()));

            let autosize = autosize::autosize(
                if self.menu.is_some() {
                    Element::from(
                        mouse_area(window)
                            .on_release(Message::CloseContextMenu)
                            .on_right_release(Message::CloseContextMenu),
                    )
                } else {
                    window.into()
                },
                AUTOSIZE_ID.clone(),
            );
            return Element::from(autosize);
'''

sub(r"            let launcher_entry = text_input::search_input\(.*?\n            return Element::from\(autosize\);\n",
    lambda m: VIEW, re.S)

HELPERS = r'''
// ---- macOS Spotlight helpers (cosmic-mac patches/launcher.py) ----
const MAC_BAR_W: f32 = 520.0;

/// Top offset of the Spotlight pill: about a third of the way down the smallest screen.
static MAC_TOP: LazyLock<f32> = LazyLock::new(|| {
    let out = std::process::Command::new("cosmic-randr").arg("list").output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    let mut scale = 1.0f32;
    let mut heights = Vec::new();
    for line in out.lines() {
        let l: String = line.chars().filter(|c| !c.is_control()).collect();
        let l = l.replace("[0m", "").replace("[1;33m", "");
        if let Some(v) = l.trim().strip_prefix("Scale:") {
            scale = v.trim().trim_end_matches('%').parse::<f32>().map(|p| p / 100.0).unwrap_or(1.0);
        } else if l.contains("(current)") {
            if let Some(h) = l.trim().split_whitespace().next().and_then(|m| m.split('x').nth(1)).and_then(|h| h.parse::<f32>().ok()) {
                heights.push(h / scale.max(0.1));
            }
        }
    }
    heights.into_iter().fold(f32::MAX, f32::min).min(1400.0).max(600.0) * 0.36
});

fn mac_circle(theme: &cosmic::Theme, hover: f32) -> button::Style {
    let t = theme.cosmic();
    let mut bg: Color = t.background(theme.transparent).base.into();
    bg.a = bg.a.max(0.92);
    let on: Color = t.on_bg_color().into();
    button::Style {
        background: Some(Color { r: bg.r + (on.r - bg.r) * hover, g: bg.g + (on.g - bg.g) * hover, b: bg.b + (on.b - bg.b) * hover, a: bg.a }.into()),
        border_radius: 999.0.into(),
        border_width: 1.0,
        border_color: if t.is_dark { Color::from_rgba(1.0, 1.0, 1.0, 0.14) } else { Color::from_rgba(0.0, 0.0, 0.0, 0.10) },
        icon_color: Some(on),
        text_color: Some(on),
        ..button::Style::new()
    }
}

fn mac_row(theme: &cosmic::Theme, selected: bool, hover: f32) -> button::Style {
    let t = theme.cosmic();
    let on: Color = t.on_bg_color().into();
    let bg = if selected {
        let mut a: Color = t.accent_color().into();
        a.a = 0.35;
        Some(a.into())
    } else if hover > 0.0 {
        Some(Color { a: hover, ..on }.into())
    } else {
        None
    };
    button::Style {
        background: bg,
        border_radius: 12.0.into(),
        outline_width: 0.0,
        icon_color: Some(on),
        text_color: Some(on),
        ..button::Style::new()
    }
}
'''
sub(r"(\nconst SCROLL_MIN: usize = \d+;\n)", lambda m: m.group(1) + HELPERS)

open(p, "w").write(s)
