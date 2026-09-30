#!/usr/bin/env python3
"""Patch cosmic-settings toward macOS System Settings (run from its source root).
Exits 1 if the code no longer matches.

- Window: patches/mac_sidebar.rs draws it like macOS - traffic lights, a see-through
  blurred sidebar (search on top, colored rounded icons, grouped, blue selection) and
  an opaque content area; COSMIC's header bar and nav bar are hidden.
- The search button in the header bar is gone (search lives in the sidebar).
"""
import os, re, shutil, sys

here = os.path.dirname(os.path.abspath(__file__))
shutil.copy(os.path.join(here, "mac_sidebar.rs"), "cosmic-settings/src/mac_sidebar.rs")

p = "cosmic-settings/src/app.rs"
s = open(p).read()

def sub(pat, rep):
    global s
    s, n = re.subn(pat, rep, s, count=1, flags=re.S)
    if n != 1:
        print(f"settings patch failed: {pat[:70]}", file=sys.stderr)
        sys.exit(1)

# child module of app.rs, so it can read the app's private fields
sub(r"(\npub struct SettingsApp \{)", r"""
#[path = "mac_sidebar.rs"]
mod mac_sidebar;
\1""")

# macOS window: no COSMIC nav bar or header bar; view() draws sidebar + content itself
sub(r"(    fn nav_model\(&self\) -> Option<&nav_bar::Model> \{\n        Some\(&self\.nav_model\)\n    \}\n)",
    r"""\1
    fn nav_bar(&self) -> Option<Element<'_, cosmic::Action<Self::Message>>> {
        None
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        // transparent window: the sidebar is see-through (blurred by the compositor)
        let t = cosmic::theme::active();
        let on: cosmic::iced::Color = t.cosmic().on_bg_color().into();
        Some(cosmic::iced::theme::Style { background_color: cosmic::iced::Color::TRANSPARENT, text_color: on, icon_color: on })
    }
""")
sub(r"(        let mut app = SettingsApp \{\n.*?\n        \};\n)",
    r"""\1        app.core.window.show_headerbar = false;
        app.core.window.content_container = false;
""")
sub(r"(        let task = Task::batch\(\[\n)",
    r"""        let mac_blur = match app.core.main_window_id() {
            Some(id) => cosmic::iced::runtime::task::effect(cosmic::iced::runtime::Action::PlatformSpecific(
                cosmic::iced::runtime::platform_specific::Action::Wayland(
                    cosmic::iced::runtime::platform_specific::wayland::Action::BlurSurface(
                        id,
                        Some(vec![cosmic::iced::Rectangle { x: 0., y: 0., width: f32::MAX, height: f32::MAX }]),
                    ),
                ),
            )),
            None => Task::none(),
        };
\1            mac_blur,
""")
sub(r"(pub enum Message \{\n)", r"\1    MacClose,\n    MacMinimize,\n    MacMaximize,\n    MacDrag,\n")
sub(r"(    fn update\(&mut self, message: Message\) -> Task<Self::Message> \{\n        match message \{\n)",
    r"""\1            Message::MacClose => return cosmic::iced::Task::done(cosmic::Action::Cosmic(cosmic::app::Action::Close)),
            Message::MacMinimize => return cosmic::iced::Task::done(cosmic::Action::Cosmic(cosmic::app::Action::Minimize)),
            Message::MacMaximize => return cosmic::iced::Task::done(cosmic::Action::Cosmic(cosmic::app::Action::Maximize)),
            Message::MacDrag => return cosmic::iced::Task::done(cosmic::Action::Cosmic(cosmic::app::Action::Drag)),
""")
sub(r"            return self\.page_container\(cosmic::widget::space\(\)\);\n        \};\n\n        container\(view\)\.into\(\)\n",
    "            return mac_sidebar::window(self, self.page_container(cosmic::widget::space()));\n        };\n\n        mac_sidebar::window(self, container(view).into())\n")

# no search button in the header: search is in the sidebar
sub(r"(    fn header_start\(&self\) -> Vec<Element<'_, Self::Message>> \{\n)        let mut widgets = Vec::new\(\);\n.*?\n        widgets\n    \}\n",
    r"\1        Vec::new()\n    }\n")

open(p, "w").write(s)

# ---------------------------------------------------------------------------
# Page header like macOS: a round back button, then the page title in bold,
# instead of COSMIC's link-style breadcrumb above a large title.
w = "cosmic-settings/src/widget/mod.rs"
t = open(w).read()
t, n1 = re.subn(
    r"(pub fn page_title<Message: 'static>\(page: &page::Info\) -> Element<'_, Message> \{\n    row::with_capacity\(2\)\n        \.push\()text::title3\(page\.title\.as_str\(\)\)\)",
    r"\1text::title4(page.title.as_str()).font(cosmic::font::bold()))", t, count=1)
t, n2 = re.subn(
    r"    let previous_button = button::icon\(icon::from_name\(\"go-previous-symbolic\"\)\)\n        \.extra_small\(\)\n        \.padding\(0\)\n        \.label\(parent_page\)\n        \.spacing\(4\)\n        \.class\(button::ButtonClass::Link\)\n        \.on_press\(on_press\);\n\n    let sub_page_header = row::with_capacity\(2\)\.push\(text::title3\(sub_page\)\);\n\n    column::with_capacity\(2\)\n        \.push\(previous_button\)\n        \.push\(sub_page_header\)\n        \.spacing\(6\)\n        \.width\(Length::Shrink\)\n        \.into\(\)",
    "    // macOS: a round back button beside the bold page title (cosmic-mac patches/settings.py).\n"
    "    let _ = parent_page;\n"
    "    let previous_button = button::icon(icon::from_name(\"go-previous-symbolic\"))\n"
    "        .padding(6)\n"
    "        .class(button::ButtonClass::Standard)\n"
    "        .on_press(on_press);\n\n"
    "    row::with_capacity(2)\n"
    "        .push(previous_button)\n"
    "        .push(text::title4(sub_page).font(cosmic::font::bold()))\n"
    "        .spacing(12)\n"
    "        .align_y(cosmic::iced::Alignment::Center)\n"
    "        .width(Length::Shrink)\n"
    "        .into()",
    t, count=1)
if n1 != 1 or n2 != 1:
    print("settings header patch failed", n1, n2, file=sys.stderr)
    sys.exit(1)
open(w, "w").write(t)

# ---------------------------------------------------------------------------
# Appearance: accent colours as a row of small round swatches beside the label,
# like macOS, instead of a grid of big squares.
a = "cosmic-settings/src/pages/desktop/appearance/mode_and_colors.rs"
t = open(a).read()
t, n3 = re.subn(
    r"(fn accent_color_palette<'a>\(.*?\n)(\s*for &color in accent \{\n\s*accent_palette_row\.push\()color_button\(\n(\s*Some\(Message::PaletteAccent\(color\.into\(\)\)\),\n\s*color\.into\(\),\n\s*cur_accent == color,\n)\s*48,\n\s*48,\n(\s*\)\);)",
    r"\1\2mac_swatch(\n\3\4", t, count=1, flags=re.S)
t, n4 = re.subn(
    r"    cosmic::iced::widget::column!\[\n        text::body\(&descriptions\[labels\[\"accent_color\"\]\]\),\n        widget::flex_row\(accent_palette_row\)\.spacing\(16\)\n    \]\n    \.spacing\(space_xxs\)",
    "    let _ = space_xxs;\n"
    "    cosmic::iced::widget::row![\n"
    "        text::body(&descriptions[labels[\"accent_color\"]]).width(Length::Fill),\n"
    "        widget::flex_row(accent_palette_row).spacing(10).align_items(Alignment::Center)\n"
    "    ]\n"
    "    .align_y(Alignment::Center)",
    t, count=1)
t, n5 = re.subn(r"(\n/// A button for selecting a color or gradient\.\n)", r'''
/// A round 22px colour swatch with a ring when selected (macOS accent picker).
fn mac_swatch<'a, Message: 'a + Clone>(on_press: Option<Message>, color: cosmic::iced::Color, selected: bool) -> Element<'a, Message> {
    let dot = container(widget::Space::new().width(Length::Fixed(22.0)).height(Length::Fixed(22.0)))
        .class(cosmic::theme::Container::custom(move |_| cosmic::iced::widget::container::Style {
            background: Some(cosmic::iced::Background::Color(color)),
            border: cosmic::iced::Border { radius: 11.0.into(), ..Default::default() },
            ..Default::default()
        }));
    let ring = container(dot)
        .padding(2)
        .class(cosmic::theme::Container::custom(move |t| cosmic::iced::widget::container::Style {
            border: cosmic::iced::Border {
                radius: 14.0.into(),
                width: if selected { 2.0 } else { 0.0 },
                color: if selected { color } else { cosmic::iced::Color::TRANSPARENT },
            },
            ..Default::default()
        }));
    button::custom(ring).padding(0).class(button::ButtonClass::Text).on_press_maybe(on_press).into()
}
\1''', t, count=1)
if (n3, n4, n5) != (1, 1, 1):
    print("settings accent patch failed", n3, n4, n5, file=sys.stderr)
    sys.exit(1)
open(a, "w").write(t)

# the custom-colour button next to the swatches: round and small too
t = open(a).read()
t, n6 = re.subn(
    r"(                    \.picker_button\(\|_\| Message::DrawerOpen\(ContextView::CustomAccent\), Some\(24\)\)\n)                    \.width\(Length::Fixed\(48\.0\)\)\n                    \.height\(Length::Fixed\(48\.0\)\),",
    r"\1                    .width(Length::Fixed(26.0))\n                    .height(Length::Fixed(26.0)),", t, count=1)
t, n7 = re.subn(
    r"(                Some\(Message::DrawerOpen\(ContextView::CustomAccent\)\),\n                c,\n                cosmic::iced::Color::from\(cur_accent\) == c,\n)                48,\n                48,\n",
    r"\1                26,\n                26,\n", t, count=1)
if (n6, n7) != (1, 1):
    print("settings custom accent patch failed", n6, n7, file=sys.stderr)
    sys.exit(1)
open(a, "w").write(t)
