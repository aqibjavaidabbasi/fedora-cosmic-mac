#!/usr/bin/env python3
"""Patch cosmic-app-library into a macOS Launchpad (run from its source root).
Exits 1 if the code no longer matches.

- patches/mac_launchpad.rs draws the whole thing: full screen over the blurred wallpaper,
  a search pill on top, 7 x 5 pages of big icons with page dots, folders (app groups)
  as glass tiles with mini icons, and a New Folder tile.
- Scrolling or swiping flips pages; clicking empty space closes it.
- App tiles get bigger icons and shorter names.
"""
import os, re, shutil, sys

here = os.path.dirname(os.path.abspath(__file__))
shutil.copy(os.path.join(here, "mac_launchpad.rs"), "src/mac_launchpad.rs")

def patch(path, edits):
    s = open(path).read()
    for pat, rep in edits:
        s, n = re.subn(pat, rep, s, count=1, flags=re.S)
        if n != 1:
            print(f"app-library patch failed in {path}: {pat[:70]}", file=sys.stderr)
            sys.exit(1)
    open(path, "w").write(s)

patch("src/app.rs", [
    # child module, so it can read the app's private fields
    (r"\nstruct CosmicAppLibrary \{\n", "\n#[path = \"mac_launchpad.rs\"]\nmod mac_launchpad;\n\nstruct CosmicAppLibrary {\n"),
    (r"(    margin: f32,\n    size: iced::Size,\n)",
     r"\1    mac_page: usize,\n    mac_wheel: f32,\n    mac_flip: Option<Instant>,\n"),
    (r"(            margin: Default::default\(\),\n)",
     r"\1            mac_page: 0,\n            mac_wheel: 0.,\n            mac_flip: None,\n"),
    (r"(    Output\(OutputEvent\),\n\})", r"    MacPage(usize),\n    MacWheel(f32),\n\1"),
    # page handling
    (r"(    fn update\(&mut self, message: Message\) -> Task<Self::Message> \{\n        match message \{\n)",
     r"""\1            Message::MacPage(p) => {
                self.mac_page = p;
            }
            Message::MacWheel(d) => {
                let now = Instant::now();
                if self.mac_flip.is_some_and(|t| now.duration_since(t) < Duration::from_millis(450)) {
                    self.mac_wheel = 0.;
                    return Task::none();
                }
                self.mac_wheel += d;
                if self.mac_wheel.abs() >= 40. {
                    let pages = mac_launchpad::pages(self);
                    if self.mac_wheel < 0. {
                        if self.mac_page + 1 < pages {
                            self.mac_page += 1;
                        }
                    } else {
                        self.mac_page = self.mac_page.saturating_sub(1);
                    }
                    self.mac_wheel = 0.;
                    self.mac_flip = Some(now);
                }
            }
"""),
    # keyboard focus moves to the page that holds the focused app
    (r"(            Message::UpdateFocused\(id\) => \{\n.*?\.unwrap_or\(0\);\n)",
     r"\1                self.mac_page = i / mac_launchpad::PER_PAGE;\n"),
    (r"(            self\.scroll_offset = 0\.0;\n            self\.cur_group = None;\n)",
     r"\1            self.mac_page = 0;\n"),
    (r"(            Message::InputChanged\(value\) => \{\n                self\.search_value = value;\n)",
     r"\1                self.mac_page = 0;\n"),
    (r"(                self\.cur_group = group;\n)", r"\1                self.mac_page = 0;\n"),
    # full screen below the top bar
    (r"(    fn layer_padding\(&self\) -> IcedMargin \{\n).*?(\n    \}\n)",
     r"""\1        IcedMargin {
            #[allow(clippy::cast_possible_truncation)]
            top: self.margin as i32,
            left: 0,
            right: 0,
            bottom: 0,
        }\2"""),
    # the Launchpad view replaces the centered window
    (r"        let cur_group = self\.current_group\(\);\n        let top_row = .*?(\n    \}\n\n    fn subscription)",
     r"        mac_launchpad::view(self)\1"),
    # always ask the compositor to blur what is behind (the wallpaper), like Launchpad
    (r"if self\.core\.system_theme\(\)\.cosmic\(\)\.frosted_system_interface \{", "if true {"),
    # scroll / swipe flips pages
    (r"(                cosmic::iced::Event::Window\(WindowEvent::Opened \{)",
     r"""                cosmic::iced::Event::Mouse(iced::mouse::Event::WheelScrolled { delta })
                    if id == SurfaceId::RESERVED =>
                {
                    Some(Message::MacWheel(match delta {
                        iced::mouse::ScrollDelta::Lines { x, y } => (if y.abs() >= x.abs() { y } else { x }) * 60.,
                        iced::mouse::ScrollDelta::Pixels { x, y } => if y.abs() >= x.abs() { y } else { x },
                    }))
                }
\1"""),
])

patch("src/widgets/application.rs", [
    (r"format!\(\"\{name:\.24\}\.\.\.\"\)", r'format!("{name:.18}...")'),
    (r"(                    icon_handle\n                        \.clone\(\)\n                        \.icon\(\)\n)                        \.width\(Length::Fixed\(72\.0\)\)\n                        \.height\(Length::Fixed\(72\.0\)\),",
     r"\1                        .width(Length::Fixed(88.0))\n                        .height(Length::Fixed(88.0)),"),
    (r"\.size\(14\.0\)", ".size(13.0)"),
    (r"\.height\(Length::Fixed\(120\.0\)\)", ".height(Length::Fixed(136.0))"),
])
