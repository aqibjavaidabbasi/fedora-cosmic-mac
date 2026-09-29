#!/usr/bin/env python3
"""Patch cosmic-app-list (the Dock's app icons) toward the macOS Dock.
Run from the cosmic-applets source root. Exits 1 if the code no longer matches.

- Icons sit close together like macOS: the padding along the Dock is cut to 30%.
- Hover magnification: icons are drawn at 80% of the slot and grow smoothly
  (fisheye, up to 1.45x) under the pointer, using the headroom above them, so
  the Dock itself never changes height.
"""
import re, sys

p = "cosmic-app-list/src/app.rs"
s = open(p).read()

def sub(pat, rep):
    global s
    s, n = re.subn(pat, rep, s, count=1)
    if n != 1:
        print(f"dock patch failed: {pat[:70]}", file=sys.stderr)
        sys.exit(1)

# --- shared hover state + fisheye curve --------------------------------------
sub(r"(\nstatic DND_FAVORITES: u64 = u64::MAX;\n)", r"""\1
// ---- macOS Dock magnification (cosmic-mac patches/dock.py) ----
const MAC_MAG: f32 = 1.45;
thread_local! {
    /// Pointer position along the Dock, in the (magnified) layout's coordinates.
    static MAC_HOVER: std::cell::Cell<Option<f32>> = const { std::cell::Cell::new(None) };
    /// Current scale of each dock item, by item id.
    static MAC_SCALE: std::cell::RefCell<FxHashMap<u32, f32>> = std::cell::RefCell::new(FxHashMap::default());
}

/// Smooth bell: MAC_MAG right under the pointer, 1.0 two and a bit icons away.
fn mac_scale_at(distance: f32, slot: f32) -> f32 {
    let r = slot * 2.2;
    if distance.abs() >= r {
        1.0
    } else {
        1.0 + (MAC_MAG - 1.0) * (1.0 + (std::f32::consts::PI * distance / r).cos()) / 2.0
    }
}
""")

# --- smaller base icon, tight spacing, headroom for growth --------------------
sub(r"        let icon_size = applet\.suggested_size\(false\)\.0;\n        let \(major_padding, cross_padding\) = applet\.suggested_padding\(false\);\n",
    "        // macOS Dock (cosmic-mac patches/dock.py): icons at 80% of the slot, the rest is\n"
    "        // headroom for hover magnification; icons close together along the Dock.\n"
    "        let full_icon = applet.suggested_size(false).0;\n"
    "        let icon_size = ((full_icon as f32) * 0.8).round() as u16;\n"
    "        let (major_padding, cross_padding) = applet.suggested_padding(false);\n"
    "        let major_padding = ((major_padding as f32) * 0.3).max(2.0) as u16;\n")
sub(r"(        \};\n)(        AppletIconData \{\n            icon_size,)",
    r"""\1        let mac_head = (full_icon - icon_size) as f32;
        let mut padding = padding;
        match applet.anchor {
            PanelAnchor::Bottom => padding[0] += mac_head,
            PanelAnchor::Top => padding[2] += mac_head,
            PanelAnchor::Left => padding[1] += mac_head,
            PanelAnchor::Right => padding[3] += mac_head,
        }
\2""")

# --- each icon grows by its scale, eating the headroom ------------------------
sub(r"(        let app_icon = AppletIconData::new\(applet\);\n)(\n        let cosmic_icon = cosmic::widget::icon\()",
    r"""\1        let mut app_icon = app_icon;
        let mac_s = MAC_SCALE.with(|m| m.borrow().get(id).copied().unwrap_or(1.0));
        let mac_grow = (app_icon.icon_size as f32 * (mac_s - 1.0)).round();
        let mac_size = app_icon.icon_size + mac_grow as u16;
        match applet.anchor {
            PanelAnchor::Bottom => app_icon.padding.top = (app_icon.padding.top - mac_grow).max(0.0),
            PanelAnchor::Top => app_icon.padding.bottom = (app_icon.padding.bottom - mac_grow).max(0.0),
            PanelAnchor::Left => app_icon.padding.right = (app_icon.padding.right - mac_grow).max(0.0),
            PanelAnchor::Right => app_icon.padding.left = (app_icon.padding.left - mac_grow).max(0.0),
        }
\2""")
sub(r"        \.size\(128\)\n        \.width\(app_icon\.icon_size\.into\(\)\)\n        \.height\(app_icon\.icon_size\.into\(\)\);",
    "        .size(128)\n        .width(mac_size.into())\n        .height(mac_size.into());")

# --- hover message ------------------------------------------------------------
sub(r"(enum Message \{\n    Wayland\(WaylandUpdate\),)", r"\1\n    MacHover(Option<f32>),")
sub(r"(    fn update\(&mut self, message: Self::Message\) -> app::Task<Self::Message> \{\n        match message \{\n)",
    r"""\1            Message::MacHover(pos) => {
                MAC_HOVER.with(|h| h.set(pos));
                return Task::none();
            }
""")

# --- per-item scales from the pointer (in un-magnified coordinates) -----------
sub(r"(        let divider_padding = match self\.core\.applet\.size \{\n)",
    r"""        {
            // Items in dock order: pinned, then the running ones on this workspace.
            let pad = app_icon.padding;
            let slot = app_icon.icon_size as f32
                + if is_horizontal { pad.left + pad.right } else { pad.top + pad.bottom }
                + app_icon.icon_spacing;
            let active_ids: Vec<u32> = self
                .active_list
                .iter()
                .filter(|d| d.toplevels.iter().any(|(t, _)| self.is_on_current_monitor_and_workspace(t)))
                .map(|d| d.id)
                .collect();
            let pinned_ids: Vec<u32> = self.pinned_list.iter().map(|d| d.id).collect();
            let gap = if !pinned_ids.is_empty() && !active_ids.is_empty() { 9.0 } else { 0.0 };
            let old = MAC_SCALE.with(|m| m.borrow().clone());
            let mut map = FxHashMap::default();
            if let Some(pointer) = MAC_HOVER.with(|h| h.get()) {
                // Map the pointer back into un-magnified coordinates using the widths
                // currently on screen, so growing icons don't move the target.
                let mut at = 0.0f32;
                let mut base = 0.0f32;
                let mut base_x = None;
                for (k, id) in pinned_ids.iter().chain(active_ids.iter()).enumerate() {
                    if k == pinned_ids.len() {
                        at += gap;
                        base += gap;
                    }
                    let w = slot + app_icon.icon_size as f32 * (old.get(id).copied().unwrap_or(1.0) - 1.0);
                    if pointer < at + w {
                        base_x = Some(base + (pointer - at).max(0.0) / w * slot);
                        break;
                    }
                    at += w;
                    base += slot;
                }
                let base_x = base_x.unwrap_or(base);
                let mut c = 0.0f32;
                for (k, id) in pinned_ids.iter().chain(active_ids.iter()).enumerate() {
                    if k == pinned_ids.len() {
                        c += gap;
                    }
                    map.insert(*id, mac_scale_at(base_x - (c + slot / 2.0), slot));
                    c += slot;
                }
            }
            MAC_SCALE.with(|m| *m.borrow_mut() = map);
        }
\1""")

# --- track the pointer over the whole dock row --------------------------------
sub(r"(        self\.core\n            \.applet\n            \.autosize_window\(content\)\n            \.limits\(limits\)\n            \.into\(\)\n    \}\n\n    fn view_window)",
    r"""        let content = mouse_area(content)
            .on_move(move |p| Message::MacHover(Some(if is_horizontal { p.x } else { p.y })))
            .on_exit(Message::MacHover(None));
\1""")

open(p, "w").write(s)
