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
