#!/usr/bin/env python3
"""Patch cosmic-panel (run from its source root). Exits 1 if the code changed.

Changing a panel's background between custom colors (or the theme default)
no longer tears the panel down and restarts every applet; it just recolors.
A full rebuild still happens when switching between Dark and Light styles,
since that changes the applets' text color.
"""
import re, sys

p = "cosmic-panel-bin/src/space_container/space_container.rs"
s = open(p).read()
s, n = re.subn(
    r"\|\| c\.background != entry\.background",
    "|| panel_bg_family(&c.background) != panel_bg_family(&entry.background)",
    s,
    count=1,
)
if n != 1:
    sys.exit(1)
s += """
/// Background changes within the same family can be applied without
/// restarting applets: custom colors and the theme default share the
/// theme's text color; Dark and Light each need their own.
fn panel_bg_family(b: &CosmicPanelBackground) -> u8 {
    match b {
        CosmicPanelBackground::Dark => 1,
        CosmicPanelBackground::Light => 2,
        _ => 0,
    }
}
"""
open(p, "w").write(s)
