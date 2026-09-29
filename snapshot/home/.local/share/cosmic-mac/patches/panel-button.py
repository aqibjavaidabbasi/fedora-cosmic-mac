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
