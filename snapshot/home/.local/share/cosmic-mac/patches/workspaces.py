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

# --- close button on each workspace thumbnail (like the x on a macOS Space): its windows move
# to the neighbouring workspace and the empty one goes away. Shown while hovered.
p = "src/view/mod.rs"
s = open(p).read()
s, n1 = re.subn(
    r"    let workspace_footer = row!\[\n        widget::space::horizontal\(\)\.width\(Length::Fixed\(32\.0\)\),",
    "    let workspace_footer = row![\n        close_workspace_button(workspace, _output),",
    s, count=1)
s, n2 = re.subn(
    r"(fn workspace_item_appearance\()",
    """fn close_workspace_button(workspace: &Workspace, output: &wl_output::WlOutput) -> cosmic::Element<'static, Msg> {
    crate::widgets::visibility_wrapper(
        widget::button::custom(
            widget::icon::from_name("window-close-symbolic")
                .symbolic(true)
                .size(16),
        )
        .padding([4, 8])
        .class(cosmic::theme::Button::Standard)
        .on_press(Msg::RemoveWorkspace(workspace.handle().clone(), output.clone())),
        workspace.has_cursor,
    )
    .into()
}

\\1""",
    s, count=1)
open(p, "w").write(s)

p = "src/main.rs"
s = open(p).read()
s, n3 = re.subn(r"(\n    TogglePinned\(ExtWorkspaceHandleV1\),)", r"\1\n    RemoveWorkspace(ExtWorkspaceHandleV1, wl_output::WlOutput),", s, count=1)
s, n4 = re.subn(
    r"(            Msg::TogglePinned\(workspace_handle\) => \{)",
    """            Msg::RemoveWorkspace(handle, output) => {
                let siblings: Vec<ExtWorkspaceHandleV1> =
                    self.workspaces.for_output(&output).map(|w| w.handle().clone()).collect();
                if let Some(idx) = siblings.iter().position(|h| *h == handle) {
                    let target = if idx > 0 { siblings.get(idx - 1) } else { siblings.get(1) };
                    if let Some(target) = target.cloned() {
                        let (pinned, active) = self
                            .workspaces
                            .for_handle(&handle)
                            .map(|w| (w.is_pinned(), w.is_active()))
                            .unwrap_or_default();
                        if pinned {
                            self.send_wayland_cmd(backend::Cmd::SetWorkspacePinned(handle.clone(), false));
                        }
                        if active {
                            self.send_wayland_cmd(backend::Cmd::ActivateWorkspace(target.clone()));
                        }
                        let moving: Vec<ExtForeignToplevelHandleV1> = self
                            .toplevels
                            .0
                            .iter()
                            .filter(|t| t.info.workspace.contains(&handle.id()))
                            .map(|t| t.handle.clone())
                            .collect();
                        for t in moving {
                            if let Some(top) = self.toplevels.for_handle_mut(&t) {
                                top.pending_move = Some(target.clone());
                            }
                            self.send_wayland_cmd(backend::Cmd::MoveToplevelToWorkspace(
                                t,
                                target.clone(),
                                output.clone(),
                            ));
                        }
                    }
                }
            }
\\1""",
    s, count=1)
open(p, "w").write(s)
if (n1, n2, n3, n4) != (1, 1, 1, 1):
    print("workspaces close button patch failed", n1, n2, n3, n4, file=sys.stderr)
    sys.exit(1)

# --- thumbnails refresh about 10 times a second, not on every frame: capturing every window and
# every workspace at full rate made the overview (and the pointer in it) crawl on a 4K screen.
p = "src/backend/wayland/screencopy.rs"
s = open(p).read()
s, n = re.subn(
    r"        self\.thread_pool\.spawn_ok\(async move \{\n            if let Some\(release\) = release \{\n                // Wait for buffer to be released by server\n                release\.await;\n            \}\n",
    """        std::thread::spawn(move || {
            if let Some(release) = release {
                // Wait for buffer to be released by server
                cosmic::iced::futures::executor::block_on(release);
            }
            std::thread::sleep(std::time::Duration::from_millis(90));
""",
    s, count=1)
if n != 1:
    print("workspaces capture throttle patch failed", file=sys.stderr)
    sys.exit(1)
open(p, "w").write(s)

# --- the row of workspace thumbnails is a sheet of glossy glass like the Settings sidebar and the
# top-bar popovers: the same tint and edge, blurred behind (the blur strips follow its corners).
p = "src/view/mod.rs"
s = open(p).read()
s, g1 = re.subn(
    r"                        background: Some\(\n                            iced::Color::from\(theme\.cosmic\(\)\.background\(theme\.transparent\)\.base\)\n                                \.into\(\),\n                        \),\n                        border: Border \{\n                            radius: theme\n                                \.cosmic\(\)\n                                \.radius_s\(\)\n                                \.map\(\|x\| if x < 4\.0 \{ x \} else \{ x \+ 8\.0 \}\)\n                                \.into\(\),\n                            \.\.Default::default\(\)\n                        \},",
    """                        background: Some({
                            let (top, bottom) = if theme.cosmic().is_dark {
                                (iced::Color::from_rgba(0.24, 0.24, 0.27, 0.80), iced::Color::from_rgba(0.13, 0.13, 0.15, 0.86))
                            } else {
                                (iced::Color::from_rgba(0.99, 0.99, 1.0, 0.80), iced::Color::from_rgba(0.90, 0.90, 0.93, 0.88))
                            };
                            iced::Background::Gradient(iced::Gradient::Linear(
                                iced::gradient::Linear::new(iced::Radians(std::f32::consts::PI))
                                    .add_stop(0.0, top)
                                    .add_stop(1.0, bottom),
                            ))
                        }),
                        border: Border {
                            radius: 16.0.into(),
                            width: 1.0,
                            color: {
                                let on: iced::Color = theme.cosmic().on_bg_color().into();
                                iced::Color { a: 0.10, ..on }
                            },
                        },""",
    s, count=1)
open(p, "w").write(s)
p = "src/main.rs"
s = open(p).read()
s, g2 = re.subn(
    r"            let rad = active\n                \.cosmic\(\)\n                \.radius_s\(\)\n                \.map\(\|x\| if x < 4\.0 \{ x \} else \{ x \+ 8\.0 \}\);\n",
    "            let _ = &active;\n            let rad = [16.0f32; 4];\n",
    s, count=1)
open(p, "w").write(s)
if (g1, g2) != (1, 1):
    print("workspaces glass patch failed", g1, g2, file=sys.stderr)
    sys.exit(1)
