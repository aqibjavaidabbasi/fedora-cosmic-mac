#!/usr/bin/env python3
"""Patch cosmic-panel-button so icon/text can be chosen per button.

Config keys looked up in order: "<panel>/<desktop-id>", "*/<desktop-id>", "<panel>".
Run from the cosmic-applets source root. Exits 1 if the code no longer matches.
"""
import re, sys

p = "cosmic-panel-button/src/lib.rs"
s = open(p).read()

pattern = re.compile(
    r"\.configs\s*\.get\(&(?P<core>core|self\.core)\.applet\.panel_type\.to_string\(\)\)\s*\.cloned\(\)\s*\.unwrap_or_default\(\)"
)
s, n = pattern.subn(lambda m: f".pick(&{m.group('core')}.applet.panel_type.to_string())", s)
if n != 2:
    sys.exit(1)
helper = '''
impl CosmicPanelButtonConfig {
    /// Per-button override: "<panel>/<desktop-id>", then "*/<desktop-id>", then "<panel>".
    fn pick(&self, panel: &str) -> IndividualConfig {
        let id = std::env::args().nth(1).unwrap_or_default();
        self.configs
            .get(&format!("{panel}/{id}"))
            .or_else(|| self.configs.get(&format!("*/{id}")))
            .or_else(|| self.configs.get(panel))
            .cloned()
            .unwrap_or_default()
    }
}
'''
s = s.replace("mod config;\n", "mod config;\n" + helper, 1)
open(p, "w").write(s)

# ---------------------------------------------------------------------------
# Per-button custom icon: `icon: Some("name-or-/path.svg")` in the same config
# entry. The Control Center's "Edit Controls" page writes it for the left-most button.
import re

c = "cosmic-panel-button/src/config.rs"
t = open(c).read()
t, n1 = re.subn(r"(pub struct IndividualConfig \{\n    pub force_presentation: Option<Override>,\n)",
                r"\1    #[serde(default)]\n    pub icon: Option<String>,\n", t, count=1)
t, n2 = re.subn(r"(force_presentation: (?:None|Some\(Override::Icon\)),\n)(\s*)\}", r"\1\2    icon: None,\n\2}", t)
if n1 != 1 or n2 != 2:
    sys.exit(1)
open(c, "w").write(t)

s = open(p).read()
s, n3 = re.subn(r"cosmic::widget::icon::from_name\(self\.desktop\.icon\.clone\(\)\.unwrap\(\)\)\s*\.handle\(\),",
                "match self.config.icon.clone().or_else(|| self.desktop.icon.clone()).unwrap_or_default() {\n"
                "                                i if i.starts_with('/') => cosmic::widget::icon::from_path(i.into()),\n"
                "                                i => cosmic::widget::icon::from_name(i).handle(),\n"
                "                            },", s, count=1)
s, n4 = re.subn(r"if self\.desktop\.icon\.is_some\(\)\n", "if (self.desktop.icon.is_some() || self.config.icon.is_some())\n", s, count=1)
if n3 != 1 or n4 != 1:
    sys.exit(1)
open(p, "w").write(s)

# ---------------------------------------------------------------------------
# macOS Dock: buttons in the bottom dock (Downloads, Trash) match the patched
# Dock app icons (patches/dock.py): 80% icons sitting 8px above the dock edge,
# no highlight behind them, and their own slice of the glass shelf (the Trash
# button rounds the shelf's right end).
s = open(p).read()
s, n5 = re.subn(
    r"(        let symbolic = icon\.symbolic;\n)",
    r"""\1        let dock = !symbolic && matches!(self.core.applet.anchor, PanelAnchor::Bottom);
        let total = suggested.1 + 2 * vertical_padding;
        let (suggested, horizontal_padding) = if dock {
            let small = ((suggested.0 as f32) * 0.8).round() as u16;
            ((small, small), ((horizontal_padding as f32) * 0.3).max(2.0) as u16)
        } else {
            (suggested, horizontal_padding)
        };
        let (pad_top, pad_bottom) = if dock { ((total - suggested.1 - 8) as f32, 8.0) } else { (0.0, 0.0) };
""", s, count=1)
s, n6 = re.subn(
    r"            \.center\(Length::Fill\),\n        \)\n        \.width\(Length::Fixed\(\(suggested\.0 \+ 2 \* horizontal_padding\) as f32\)\)\n        \.height\(Length::Fixed\(\(suggested\.1 \+ 2 \* vertical_padding\) as f32\)\)\n        \.class\(cosmic::theme::Button::AppletIcon\)",
    "            .padding([pad_top, 0.0, pad_bottom, 0.0])\n"
    "            .center(Length::Fill),\n        )\n"
    "        .width(Length::Fixed((suggested.0 + 2 * horizontal_padding) as f32))\n"
    "        .height(Length::Fixed(if dock { total as f32 } else { (suggested.1 + 2 * vertical_padding) as f32 }))\n"
    "        .class(if dock { mac_clear_button() } else { cosmic::theme::Button::AppletIcon })",
    s, count=1)
s, n7 = re.subn(r"        autosize::autosize\(\n            if ",
    "        autosize::autosize(\n            mac_dock_shelf(&self.core.applet, std::env::args().nth(1).unwrap_or_default().ends_with(\".Trash\"), if ", s, count=1)
s, n8 = re.subn(r"(                    \.into\(\)\n            \})(,\n            AUTOSIZE_MAIN_ID\.clone\(\),)", r"\1)\2", s, count=1)
s, n9 = re.subn(r"(\nimpl Button \{\n)", r"""
/// A Dock button draws no highlight behind its icon, like macOS.
fn mac_clear_button() -> cosmic::theme::Button {
    let clear = |mut st: cosmic::widget::button::Style| {
        st.background = None;
        st.border_width = 0.0;
        st.outline_width = 0.0;
        st
    };
    let base = |f: bool, t: &cosmic::Theme| {
        cosmic::widget::button::Catalog::active(t, f, false, &cosmic::theme::Button::AppletIcon)
    };
    cosmic::theme::Button::Custom {
        active: Box::new(move |f, t| clear(base(f, t))),
        hovered: Box::new(move |f, t| clear(base(f, t))),
        pressed: Box::new(move |f, t| clear(base(f, t))),
        disabled: Box::new(move |t| clear(base(false, t))),
    }
}

/// In the bottom Dock, draw this button's slice of the glass shelf under it.
fn mac_dock_shelf<'a>(applet: &cosmic::applet::Context, round_right: bool, content: cosmic::Element<'a, Msg>) -> cosmic::Element<'a, Msg> {
    if !matches!(applet.anchor, PanelAnchor::Bottom) {
        return content;
    }
    let icon = ((applet.suggested_size(false).0 as f32) * 0.8).round();
    let shelf = cosmic::iced::widget::Column::new()
        .push(space::vertical().height(Length::Fill))
        .push(
            cosmic::widget::container(space::horizontal())
                .width(Length::Fill)
                .height(Length::Fixed(icon + 16.0))
                .class(cosmic::theme::Container::custom(move |t| {
                    let mut bg: cosmic::iced::Color = t.cosmic().background(false).base.into();
                    bg.a = 0.55;
                    let r = if round_right { 18.0 } else { 0.0 };
                    cosmic::iced::widget::container::Style {
                        background: Some(cosmic::iced::Background::Color(bg)),
                        border: cosmic::iced::Border { radius: [0.0, r, r, 0.0].into(), ..Default::default() },
                        ..Default::default()
                    }
                })),
        )
        .width(Length::Fill)
        .height(Length::Fill);
    cosmic::iced::widget::Stack::new().push(content).push_under(shelf).into()
}
\1""", s, count=1)
if (n5, n6, n7, n8, n9) != (1, 1, 1, 1, 1):
    print("panel-button dock patch failed", (n5, n6, n7, n8, n9), file=sys.stderr)
    sys.exit(1)
open(p, "w").write(s)
