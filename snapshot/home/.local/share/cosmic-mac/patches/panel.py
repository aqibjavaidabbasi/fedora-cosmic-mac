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
# A panel changing priority (the Dock toggling autohide when a window maximizes)
# only forces a rebuild of panels sharing its screen, not the other screens' Docks
# or bars.
s, n = re.subn(
    r"(\|\| c\.name != entry\.name\n\s+&& Some\(c\.anchor\) != opposite_anchor)",
    r"|| c.name != entry.name && c.output == entry.output && c.anchor == entry.anchor\n                && Some(c.anchor) != opposite_anchor",
    s, count=1)
s, n2 = re.subn(
    r"\|\| c\.name != entry\.name && old_priority != new_priority && c\.anchor == entry\.anchor",
    "|| c.name != entry.name && c.output == entry.output && old_priority != new_priority && c.anchor == entry.anchor",
    s, count=1)
if (n, n2) != (1, 1):
    sys.exit(1)
open(p, "w").write(s)

# Turning autohide on in place (the Dock hiding for a maximized app) must also give
# back the screen space the panel reserved while it was visible; before, the panel
# was rebuilt and so started without it.
p2 = "cosmic-panel-bin/src/space/panel_space.rs"
t = open(p2).read()
t, n = re.subn(
    r"(        // try to force rearrangement\n)",
    """        if config.autohide_enabled() && !self.config.autohide_enabled() && !config.exclusive_zone {
            if let Some(l) = self.layer.as_ref() {
                l.set_exclusive_zone(-1);
                needs_commit = true;
            }
        }

\\1""",
    t, count=1)
if n != 1:
    sys.exit(1)
open(p2, "w").write(t)
