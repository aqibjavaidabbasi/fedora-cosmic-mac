#!/usr/bin/env python3
"""Patch cosmic-workspaces (the overview). Run from its source root; exits 1 if the code changed.

Centers the row (or column) of workspace thumbnails, like Mission Control's
Spaces bar, instead of packing it to the start of the screen.
"""
import re, sys

p = "src/view/mod.rs"
s = open(p).read()
s, n = re.subn(
    r"let sidebar_entries_container =\n(\s*)widget::container\(crate::widgets::workspace_bar\(sidebar_entries, axis\)\)\.padding\(8\.0\);",
    r"""let sidebar_entries_container =
\1widget::container(crate::widgets::workspace_bar(sidebar_entries, axis)).padding(8.0);
    let sidebar_entries_container = match layout {
        WorkspaceLayout::Horizontal => sidebar_entries_container
            .width(Length::Fill)
            .align_x(cosmic::iced::Alignment::Center),
        WorkspaceLayout::Vertical => sidebar_entries_container
            .height(Length::Fill)
            .align_y(cosmic::iced::Alignment::Center),
    };""",
    s,
    count=1,
)
if n != 1:
    sys.exit(1)
open(p, "w").write(s)
