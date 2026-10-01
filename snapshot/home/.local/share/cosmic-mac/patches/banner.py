#!/usr/bin/env python3
"""Patch cosmic-notifications (the banner daemon) toward macOS (run from its source root).
Exits 1 if the code no longer matches.

- patches/mac_banner.rs draws each banner as a rounded frosted card: app icon, small grey
  app name, bold title, message, close button, with a soft drop shadow around it.
- Banners are wider (344px) and everything scales with the display: bigger text and
  icons on 2K and 4K screens (read from cosmic-randr for the output being used).
- The popup is no longer clipped to a rounded rectangle, so the shadow can show.
"""
import os, re, shutil, sys

here = os.path.dirname(os.path.abspath(__file__))
shutil.copy(os.path.join(here, "mac_banner.rs"), "src/mac_banner.rs")

def patch(path, edits, count=1):
    s = open(path).read()
    for pat, rep, n_expected in edits:
        s, n = re.subn(pat, rep, s, flags=re.S | re.M)
        if n != n_expected:
            print(f"banner patch failed in {path}: {pat[:70]} ({n} != {n_expected})", file=sys.stderr)
            sys.exit(1)
    open(path, "w").write(s)

patch("src/main.rs", [(r"^mod app;\n", "mod app;\nmod mac_banner;\n", 1)])

patch("src/app.rs", [
    # re-read the display size before each banner
    (r"(    fn push_notification\()\n        &mut self,\n        notification: Notification,\n    \) -> Task<<CosmicNotifications as cosmic::app::Application>::Message> \{\n",
     r"""\1
        &mut self,
        notification: Notification,
    ) -> Task<<CosmicNotifications as cosmic::app::Application>::Message> {
        crate::mac_banner::refresh(self.anchor.as_ref().and_then(|a| a.1.as_deref()));
""", 1),
    # surface and popup widths follow the banner width
    (r"size: Some\(\(Some\(300\), Some\(1\)\)\)", "size: Some((Some(crate::mac_banner::width() as u32), Some(1)))", 1),
    (r"\.min_width\(300\.0\)", ".min_width(crate::mac_banner::width())", 3),
    (r"\.max_width\(300\.0\)", ".max_width(crate::mac_banner::width())", 3),
    (r"width: 300,", "width: crate::mac_banner::width() as i32,", 1),
    # the card
    (r"                                let app_name = text::caption\(if n\.app_name\.len\(\) > 24 \{.*?                                \(n\.id, e\)\n",
     """                                let body_elem = Element::from(
                                    rich_text(html_to_spans(&n.body)).size(13.0 * crate::mac_banner::factor()),
                                )
                                .map(|_: ()| Message::Ignore);
                                let e = crate::mac_banner::card(
                                    n.notification_icon(),
                                    n.app_name.lines().next().unwrap_or_default().chars().take(26).collect(),
                                    n.summary.lines().next().unwrap_or_default().to_string(),
                                    body_elem,
                                    Message::Dismissed(n.id),
                                    Message::ActivateNotification(n.id),
                                );
                                (n.id, e)
""", 1),
    (r"                        let card_list = cosmic::widget::cards\(.*?\.width\(Length::Fixed\(300\.\)\);\n",
     "                        let card_list = notif_elems.into_iter().next().unwrap_or_else(|| space::horizontal().into());\n", 1),
    (r"\.max_width\(300\.\)", ".max_width(crate::mac_banner::width())", 1),
    # banners sit close to the bar and to each other: each surface carries shadow room around
    # its card, so pull them together by that much (first banner, then the stacked ones)
    (r"(\n\s+)\(0, -8\)(\n\s+\} else \{\n\s+)\(0, 8\)", r"\1(0, crate::mac_banner::pad() as i32 - 8)\2(0, 8 - crate::mac_banner::pad() as i32)", 2),
    (r"(let \(p_id, _, p_size\) = self\.popups\.last\(\)\.unwrap\(\);.*?)\(0, crate::mac_banner::pad\(\) as i32 - 8\)(.*?)\(0, 8 - crate::mac_banner::pad\(\) as i32\)",
     r"\1(0, 2 * crate::mac_banner::pad() as i32 - 10)\2(0, 10 - 2 * crate::mac_banner::pad() as i32)", 1),
    # no rounded clipping of the popup: the card draws its own corners and shadow
    (r"(            tasks\.push\(\n                corner_radius\(\n                    p_id,\n).*?(                \)\n                \.discard\(\),\n            \);)",
     r"\1                    None,\n\2", 1),
    (r"(                tasks\.push\(\n                    corner_radius\(\n                        id,\n).*?(                    \)\n                    \.discard\(\),\n                \);)",
     r"\1                        None,\n\2", 1),
])
