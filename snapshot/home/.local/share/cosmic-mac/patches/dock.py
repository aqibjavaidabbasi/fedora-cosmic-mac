#!/usr/bin/env python3
"""Patch cosmic-app-list (the Dock's app icons) into the macOS Dock.
Run from the cosmic-applets source root. Exits 1 if the code no longer matches.

The Dock panel itself is fully transparent; each dock applet draws its own slice
of a glass "shelf" along the bottom of its surface (the Launchpad button rounds the
left end, the Trash button the right end). Everything above the shelf is empty
headroom, so hovered icons can grow up out of the shelf like on macOS.

- Icons at 80% of the panel's icon size, sitting 8px above the shelf's bottom,
  with 8px of shelf above them; icons close together along the Dock.
- No square hover/selected highlight behind icons (macOS has none).
- Hover magnification in steps (1.45x under the pointer, 1.2x / 1.07x for the
  neighbours). Sizes only change when the pointer moves to another icon, which
  keeps it smooth.
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

# --- shared hover state, magnification steps, shelf style ---------------------
sub(r"(\nstatic DND_FAVORITES: u64 = u64::MAX;\n)", r"""\1
// ---- macOS Dock (cosmic-mac patches/dock.py) ----
thread_local! {
    /// Pointer position along the Dock, in the (magnified) layout's coordinates.
    static MAC_HOVER: std::cell::Cell<Option<f32>> = const { std::cell::Cell::new(None) };
    /// Current scale of each dock item, by item id.
    static MAC_SCALE: std::cell::RefCell<FxHashMap<u32, f32>> = std::cell::RefCell::new(FxHashMap::default());
    /// Where each item's scale is heading; MAC_SCALE eases toward it every frame.
    static MAC_TARGET: std::cell::RefCell<FxHashMap<u32, f32>> = std::cell::RefCell::new(FxHashMap::default());
    /// Set while the scales are still easing (keeps the frame ticker running).
    static MAC_ANIM: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Magnification by distance (in icons) from the hovered one.
fn mac_step(d: usize) -> f32 {
    match d {
        0 => 1.45,
        1 => 1.2,
        2 => 1.07,
        _ => 1.0,
    }
}

/// Height of the glass shelf for a given icon size: 8px below and above the icon.
pub fn mac_shelf_height(icon: u16) -> f32 {
    icon as f32 + 16.0
}

/// The glass shelf slice: translucent theme background, rounded on the chosen ends.
pub fn mac_shelf(round_left: bool, round_right: bool) -> theme::Container<'static> {
    theme::Container::custom(move |t| {
        let c = t.cosmic();
        let mut bg: iced::Color = c.background(false).base.into();
        bg.a = 0.55;
        let r = 18.0;
        container::Style {
            background: Some(Background::Color(bg)),
            border: Border {
                radius: [
                    if round_left { r } else { 0.0 },
                    if round_right { r } else { 0.0 },
                    if round_right { r } else { 0.0 },
                    if round_left { r } else { 0.0 },
                ]
                .into(),
                width: 0.0,
                color: iced::Color::TRANSPARENT,
            },
            ..Default::default()
        }
    })
}
""")

# --- icon size and padding: 80% icons, 8px above the shelf bottom -------------
sub(r"        let icon_size = applet\.suggested_size\(false\)\.0;\n        let \(major_padding, cross_padding\) = applet\.suggested_padding\(false\);\n",
    "        // macOS Dock (cosmic-mac patches/dock.py): icons at 80% of the panel's icon size,\n"
    "        // close together; the space above them is headroom for magnification.\n"
    "        let full_icon = applet.suggested_size(false).0;\n"
    "        let icon_size = ((full_icon as f32) * 0.8).round() as u16;\n"
    "        let (major_padding, cross_padding) = applet.suggested_padding(false);\n"
    "        let major_padding = ((major_padding as f32) * 0.3).max(2.0) as u16;\n")
sub(r"(        \};\n)(        AppletIconData \{\n            icon_size,)",
    r"""\1        // Surface thickness stays full_icon + 2 * cross padding; the icon (plus its
        // 5px running-dot row) sits 3px from the dock's outer edge, the rest is headroom.
        let mac_total = (full_icon + 2 * cross_padding) as f32;
        let mac_head = (mac_total - icon_size as f32 - 5.0 - 3.0).max(0.0);
        let mut padding = padding;
        match applet.anchor {
            PanelAnchor::Bottom => { padding[0] = mac_head; padding[2] = 3.0; }
            PanelAnchor::Top => { padding[2] = mac_head; padding[0] = 3.0; }
            PanelAnchor::Left => { padding[1] = mac_head; padding[3] = 3.0; }
            PanelAnchor::Right => { padding[3] = mac_head; padding[1] = 3.0; }
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

# --- no square highlight behind icons -----------------------------------------
sub(r"fn app_list_icon_style\(selected: bool\) -> cosmic::theme::Button \{\n",
    "fn app_list_icon_style(_selected: bool) -> cosmic::theme::Button {\n"
    "    // macOS Dock: no background behind icons in any state (cosmic-mac patches/dock.py).\n"
    "    let clear = |mut st: button::Style| {\n"
    "        st.background = None;\n"
    "        st.border_width = 0.0;\n"
    "        st.outline_width = 0.0;\n"
    "        st\n"
    "    };\n"
    "    return Button::Custom {\n"
    "        active: Box::new(move |f, t| clear(button::Catalog::active(t, f, false, &Button::AppletIcon))),\n"
    "        hovered: Box::new(move |f, t| clear(button::Catalog::active(t, f, false, &Button::AppletIcon))),\n"
    "        disabled: Box::new(move |t| clear(button::Catalog::disabled(t, &Button::AppletIcon))),\n"
    "        pressed: Box::new(move |f, t| clear(button::Catalog::active(t, f, false, &Button::AppletIcon))),\n"
    "    };\n"
    "    #[allow(unreachable_code)]\n"
    "    let selected = _selected;\n")

# --- hover message ------------------------------------------------------------
sub(r"(enum Message \{\n    Wayland\(WaylandUpdate\),)", r"\1\n    MacHover(Option<f32>),\n    MacPointer(window::Id, Option<cosmic::iced::Point>),\n    MacTick,")
sub(r"(    fn update\(&mut self, message: Self::Message\) -> app::Task<Self::Message> \{\n        match message \{\n)",
    r"""\1            Message::MacHover(pos) => {
                MAC_HOVER.with(|h| h.set(pos));
                return Task::none();
            }
            Message::MacPointer(id, pos) => {
                // Raw pointer events of the dock window: a mouse_area misses them once an
                // icon button under the pointer handles them, which left the magnification stuck.
                if self.core.main_window_id() == Some(id) {
                    let horizontal = matches!(self.core.applet.anchor, PanelAnchor::Top | PanelAnchor::Bottom);
                    MAC_HOVER.with(|h| h.set(pos.map(|p| if horizontal { p.x } else { p.y })));
                    MAC_ANIM.with(|a| a.set(true));
                }
                return Task::none();
            }
            Message::MacTick => {
                // ease every item's scale toward its target (about 60 times a second)
                let still = MAC_SCALE.with(|cur| {
                    MAC_TARGET.with(|tg| {
                        let mut c = cur.borrow_mut();
                        let t = tg.borrow();
                        let mut ids: Vec<u32> = c.keys().chain(t.keys()).copied().collect();
                        ids.sort_unstable();
                        ids.dedup();
                        let mut still = false;
                        for id in ids {
                            let a = c.get(&id).copied().unwrap_or(1.0);
                            let b = t.get(&id).copied().unwrap_or(1.0);
                            let mut n = a + (b - a) * 0.38;
                            if (n - b).abs() < 0.004 {
                                n = b;
                            } else {
                                still = true;
                            }
                            if (n - 1.0).abs() < 0.004 {
                                c.remove(&id);
                            } else {
                                c.insert(id, n);
                            }
                        }
                        still
                    })
                });
                MAC_ANIM.with(|a| a.set(still));
                return Task::none();
            }
""")

# --- per-item scales from the hovered icon ------------------------------------
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
            let ids: Vec<u32> = pinned_ids.iter().chain(active_ids.iter()).copied().collect();
            let old = MAC_SCALE.with(|m| m.borrow().clone());
            let mut map = FxHashMap::default();
            if let Some(pointer) = MAC_HOVER.with(|h| h.get()) {
                // Which icon is under the pointer, measured with the widths now on screen.
                let mut at = 0.0f32;
                let mut hovered = None;
                for (k, id) in ids.iter().enumerate() {
                    if k == pinned_ids.len() {
                        at += gap;
                    }
                    at += slot + app_icon.icon_size as f32 * (old.get(id).copied().unwrap_or(1.0) - 1.0);
                    if pointer < at {
                        hovered = Some(k);
                        break;
                    }
                }
                if let Some(h) = hovered {
                    for (k, id) in ids.iter().enumerate() {
                        map.insert(*id, mac_step(k.abs_diff(h)));
                    }
                }
            }
            MAC_TARGET.with(|m| *m.borrow_mut() = map);
        }
\1""")

# --- track the pointer; draw the shelf under the icons -------------------------
sub(r"(        self\.core\n            \.applet\n            \.autosize_window\(content\)\n            \.limits\(limits\)\n            \.into\(\)\n    \}\n\n    fn view_window)",
    r"""        let shelf_h = mac_shelf_height(app_icon.icon_size);
        let shelf: Element<_> = if is_horizontal {
            Column::new()
                .push(vertical_space().height(Length::Fill))
                .push(container(horizontal_space()).width(Length::Fill).height(Length::Fixed(shelf_h)).class(mac_shelf(false, false)))
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            Row::new()
                .push(container(vertical_space()).height(Length::Fill).width(Length::Fixed(shelf_h)).class(mac_shelf(false, false)))
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };
        let content = iced::widget::Stack::new().push(content).push_under(shelf);
\1""")

# --- the favorites/running divider is a short line on the shelf, not a full-height rule
sub(r"                container\(vertical_rule\(1\)\)\n                    \.height\(Length::Fill\)\n                    \.padding\(\[divider_padding, 0\]\)\n                    \.into\(\),",
    r"""                {
                    let shelf_h = mac_shelf_height(app_icon.icon_size);
                    let line = container(vertical_rule(1)).height(Length::Fixed((shelf_h * 0.6).round()));
                    Column::new()
                        .push(vertical_space().height(Length::Fill))
                        .push(container(line).height(Length::Fixed(shelf_h)).align_y(Alignment::Center).padding([0, divider_padding]))
                        .height(Length::Fill)
                        .into()
                },""")

sub(r"(                cosmic::iced::core::Event::Mouse\(\n                    cosmic::iced::core::mouse::Event::ButtonPressed\(_\),\n                \) => Some\(Message::Pressed\(id\)\),\n)",
    r"""\1                cosmic::iced::core::Event::Mouse(cosmic::iced::core::mouse::Event::CursorMoved { position }) => {
                    Some(Message::MacPointer(id, Some(position)))
                }
                cosmic::iced::core::Event::Mouse(cosmic::iced::core::mouse::Event::CursorLeft) => {
                    Some(Message::MacPointer(id, None))
                }
""")

sub(r"(    fn subscription\(&self\) -> Subscription<Message> \{\n        Subscription::batch\(\[\n)",
    r"""\1            if MAC_ANIM.with(|a| a.get()) {
                cosmic::iced::time::every(Duration::from_millis(16)).map(|_| Message::MacTick)
            } else {
                Subscription::none()
            },
""")

open(p, "w").write(s)
