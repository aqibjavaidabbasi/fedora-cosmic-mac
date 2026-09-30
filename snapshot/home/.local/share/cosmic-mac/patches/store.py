#!/usr/bin/env python3
"""Patch cosmic-store (COSMIC Store) into the macOS App Store (run from its source root).
Exits 1 if the code no longer matches.

- Renamed "App Store".
- patches/mac_store.rs draws the window like the System Settings patch: traffic lights,
  a frosted blurred sidebar (search on top, colored rounded icons, grouped pages,
  soft selection), no COSMIC header bar; a thin toolbar over the content keeps the back
  button, busy spinner and repositories menu.
- Discover (Explore) is laid out like the Mac App Store: featured banner, three
  editorial cards, then shelves of apps with GET / OPEN buttons.
- App cards elsewhere (categories, search) get a soft Mac tile instead of COSMIC's card.
"""
import os, re, shutil, sys

here = os.path.dirname(os.path.abspath(__file__))
shutil.copy(os.path.join(here, "mac_store.rs"), "src/mac_store.rs")

def patch(path, edits):
    s = open(path).read()
    for pat, rep in edits:
        s, n = re.subn(pat, rep, s, count=1, flags=re.S)
        if n != 1:
            print(f"store patch failed in {path}: {pat[:70]}", file=sys.stderr)
            sys.exit(1)
    open(path, "w").write(s)

BLUR = """cosmic::iced::runtime::task::effect(cosmic::iced::runtime::Action::PlatformSpecific(
                cosmic::iced::runtime::platform_specific::Action::Wayland(
                    cosmic::iced::runtime::platform_specific::wayland::Action::BlurSurface(
                        *window_id,
                        Some(vec![cosmic::iced::Rectangle { x: 0., y: 0., width: f32::MAX, height: f32::MAX }]),
                    ),
                ),
            ))"""

patch("src/main.rs", [
    (r"(\nmod nav;\n)", r"\1mod mac_store;\n"),
    (r"(\npub enum Message \{\n)", r"\1    MacClose,\n    MacMinimize,\n    MacMaximize,\n    MacDrag,\n    MacNav(widget::nav_bar::Id),\n"),
    # macOS window: no header bar; the app draws sidebar + content itself
    (r"(        match app\.mode \{\n            Mode::Normal => )\{\}",
     r"\1{\n                app.core.window.show_headerbar = false;\n                app.core.window.content_container = false;\n            }"),
    (r"(    fn nav_model\(&self\) -> Option<&widget::nav_bar::Model> \{)",
     r"""    fn nav_bar(&self) -> Option<Element<'_, cosmic::Action<Self::Message>>> {
        // the macOS sidebar is drawn by mac_store::window
        None
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        if !matches!(self.mode, Mode::Normal) {
            return None;
        }
        // transparent window: the sidebar is see-through (blurred by the compositor)
        let t = cosmic::theme::active();
        let on: cosmic::iced::Color = t.cosmic().on_bg_color().into();
        Some(cosmic::iced::theme::Style { background_color: cosmic::iced::Color::TRANSPARENT, text_color: on, icon_color: on })
    }

\1"""),
    # ask for the blur with every title update (the first request can be too early)
    (r"(    fn update_title\(&mut self\) -> Task<Message> \{\n        if let Some\(window_id\) = &self\.core\.main_window_id\(\) \{\n)            self\.set_window_title\(fl!\(\"app-name\"\), \*window_id\)\n",
     r'\1            Task::batch([self.set_window_title(fl!("app-name"), *window_id), ' + BLUR + r'])\n'),
    # keep asking for the blur during the first updates (the first request can be too early)
    (r"(    fn update\(&mut self, message: Self::Message\) -> Task<Message> \{\n)        self\.handle_update\(message\)\n",
     r"\1        let task = self.handle_update(message);\n        Task::batch([task, mac_store::blur_task(self)])\n"),
    # the back button moves into the toolbar over the content
    (r"                        \.push_maybe\(self\.back_button\(\)\)\n", ""),
    (r"(        // Uncomment to debug layout:\n        //content\.explain\(cosmic::iced::Color::WHITE\)\n)        content\n",
     r"\1        match self.mode {\n            Mode::Normal => mac_store::window(self, content),\n            _ => content,\n        }\n"),
])

patch("src/update.rs", [
    (r"(\n        match message \{\n)",
     r"""\1            Message::MacClose => return Task::done(cosmic::Action::Cosmic(cosmic::app::Action::Close)),
            Message::MacMinimize => return Task::done(cosmic::Action::Cosmic(cosmic::app::Action::Minimize)),
            Message::MacMaximize => return Task::done(cosmic::Action::Cosmic(cosmic::app::Action::Maximize)),
            Message::MacDrag => return Task::done(cosmic::Action::Cosmic(cosmic::app::Action::Drag)),
            Message::MacNav(id) => return <Self as cosmic::Application>::on_nav_select(self, id),
"""),
])

patch("src/view.rs", [
    (r"(                                \} else \{\n)                                    let explore_pages = ExplorePage::all\(\);\n.*?\n                                    column\.into\(\)\n(                                \}\n                            \}\n                        \}\n                    \}\n                    NavPage::Installed)",
     r"\1                                    crate::mac_store::discover(self, size.width)\n\2"),
])

patch("src/search.rs", [
    (r"(        \.height\(Length::Fixed\(48\.0 \+ \(spacing\.space_xxs as f32\) \* 2\.0\)\)\n        \.padding\(\[spacing\.space_xxs, spacing\.space_s\]\)\n)        \.class\(theme::Container::Card\)",
     r"""\1        .class(theme::Container::custom(|t| {
            // soft Mac tile instead of COSMIC's card
            let c = t.cosmic();
            let on: cosmic::iced::Color = c.on_bg_color().into();
            cosmic::iced::widget::container::Style {
                background: Some(cosmic::iced::Background::Color(cosmic::iced::Color { a: if c.is_dark { 0.06 } else { 0.035 }, ..on })),
                border: cosmic::iced::Border { radius: 12.0.into(), width: 1.0, color: cosmic::iced::Color { a: 0.06, ..on } },
                ..Default::default()
            }
        }))"""),
])

patch("i18n/en/cosmic_store.ftl", [(r"app-name = COSMIC Store\n", "app-name = App Store\n")])
s = open("i18n/en/cosmic_store.ftl").read()
if "app-name = App Store" not in s:
    sys.exit(1)
patch("res/com.system76.CosmicStore.desktop", [(r"\nName=COSMIC Store\n", "\nName=App Store\n")])
