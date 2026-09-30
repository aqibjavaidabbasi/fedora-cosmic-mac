#!/usr/bin/env python3
"""Patch cosmic-files (COSMIC Files) toward the macOS Finder (run from its source root).
Exits 1 if the code no longer matches.

- patches/mac_finder.rs draws the window: no COSMIC header bar, traffic lights on a
  frosted sidebar, and a Finder toolbar (back/forward, bold folder name, grid/list
  switch, search, menus) over the files.
- The path bar under the toolbar loses its own back/forward buttons (they are in
  the toolbar now).
- The desktop (icons on the wallpaper) is not changed.
"""
import os, re, shutil, sys

here = os.path.dirname(os.path.abspath(__file__))
shutil.copy(os.path.join(here, "mac_finder.rs"), "src/mac_finder.rs")

def patch(path, edits):
    s = open(path).read()
    for pat, rep in edits:
        s, n = re.subn(pat, rep, s, count=1, flags=re.S)
        if n != 1:
            print(f"files patch failed in {path}: {pat[:70]}", file=sys.stderr)
            sys.exit(1)
    open(path, "w").write(s)

patch("src/app.rs", [
    # child module, so it can read the app's private fields
    (r"\npub struct App \{\n", "\n#[path = \"mac_finder.rs\"]\nmod mac_finder;\n\npub struct App {\n"),
    (r"(\npub enum Message \{\n)", r"\1    MacMinimize,\n    MacMaximize,\n    MacDrag,\n"),
    (r"(    fn update\(&mut self, message: Self::Message\) -> Task<Self::Message> \{\n)",
     r"""\1        match &message {
            Message::MacMinimize => return Task::done(cosmic::Action::Cosmic(cosmic::app::Action::Minimize)),
            Message::MacMaximize => return Task::done(cosmic::Action::Cosmic(cosmic::app::Action::Maximize)),
            Message::MacDrag => return Task::done(cosmic::Action::Cosmic(cosmic::app::Action::Drag)),
            _ => {}
        }
"""),
    (r"(            Message::WindowNew => match env::current_exe\(\) \{)",
     r"            Message::MacMinimize | Message::MacMaximize | Message::MacDrag => {}\n\1"),
    # Finder window: no header bar, the app draws sidebar + toolbar itself
    (r"(            Mode::App => \{\n                core\.window\.show_context = flags\.config\.show_details;\n)",
     r"\1                core.window.show_headerbar = false;\n                core.window.content_container = false;\n"),
    (r"(    fn nav_model\(&self\) -> Option<&segmented_button::SingleSelectModel> \{)",
     r"""    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        // transparent window, so the sidebar and content draw their own backgrounds
        if !matches!(self.mode, Mode::App) {
            return None;
        }
        let t = cosmic::theme::active();
        let on: cosmic::iced::Color = t.cosmic().on_bg_color().into();
        Some(cosmic::iced::theme::Style { background_color: cosmic::iced::Color::TRANSPARENT, text_color: on, icon_color: on })
    }

\1"""),
    (r"(        let mut nav = nav\.into_container\(\);\n)",
     r"\1        nav = nav.class(cosmic::theme::Container::Transparent);\n"),
    (r"        Some\(Element::from\(\n            nav\.width\(Length::Shrink\)\.height\(Length::Fill\),\n        \)\)",
     "        Some(mac_finder::sidebar(Element::from(\n            nav.width(Length::Fixed(210.0)).height(Length::Fill),\n        )))"),
    (r"        let content: Element<_> = tab_column\.into\(\);\n",
     "        let content: Element<_> = if matches!(self.mode, Mode::App) {\n"
     "            mac_finder::content(self, tab_column.into())\n"
     "        } else {\n"
     "            tab_column.into()\n"
     "        };\n"),
])

patch("src/tab.rs", [
    (r"        row = row\.push\(prev_button\);\n", "        let _ = prev_button;\n"),
    (r"        row = row\.push\(next_button\);\n", "        let _ = next_button;\n"),
])
