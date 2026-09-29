#!/usr/bin/env python3
"""Patch cosmic-settings toward macOS System Settings (run from its source root).
Exits 1 if the code no longer matches.

- Sidebar: patches/mac_sidebar.rs replaces COSMIC's nav bar with a flat macOS list
  (search field on top, colored rounded icons, grouped, blue selection).
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

# our sidebar instead of the stock nav bar
sub(r"(    fn nav_model\(&self\) -> Option<&nav_bar::Model> \{\n        Some\(&self\.nav_model\)\n    \}\n)",
    r"""\1
    fn nav_bar(&self) -> Option<Element<'_, cosmic::Action<Self::Message>>> {
        if !self.core().nav_bar_active() {
            return None;
        }
        Some(mac_sidebar::view(self).map(cosmic::Action::App))
    }
""")

# no search button in the header: search is in the sidebar
sub(r"(    fn header_start\(&self\) -> Vec<Element<'_, Self::Message>> \{\n)        let mut widgets = Vec::new\(\);\n.*?\n        widgets\n    \}\n",
    r"\1        Vec::new()\n    }\n")

open(p, "w").write(s)
