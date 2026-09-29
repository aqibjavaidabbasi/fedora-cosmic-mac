#!/usr/bin/env python3
"""Patch cosmic-applet-notifications into a macOS-style Notification Center.

The panel button shows the date and time (replacing the separate clock applet),
with a small dot when there are unread notifications (a moon in Do Not Disturb).
Its popup shows a month calendar above Do Not Disturb and the notifications.
Run from the cosmic-applets source root. Exits 1 if the code changed.
"""
import re, sys

def patch(path, pairs):
    s = open(path).read()
    for pat, rep in pairs:
        s, n = re.subn(pat, rep, s, count=1, flags=re.S)
        if n != 1:
            print(f"patch failed in {path}: {pat[:70]}", file=sys.stderr)
            sys.exit(1)
    open(path, "w").write(s)

patch("cosmic-applet-notifications/Cargo.toml", [
    (r'(url = "[^"]*"\n)', r'\1jiff = "0.2"\n'),
])

L = "cosmic-applet-notifications/src/lib.rs"
patch(L, [
    # state
    (r"(    notifications_tx: Option<Sender<notifications::Input>>,\n\})",
     "    notifications_tx: Option<Sender<notifications::Input>>,\n"
     "    now: jiff::Zoned,\n    cal_month: jiff::civil::Date,\n    military_time: bool,\n}"),
    (r"(            notifications_tx: None,\n        \};)",
     "            notifications_tx: None,\n"
     "            now: jiff::Zoned::now(),\n"
     "            cal_month: jiff::Zoned::now().date().first_of_month(),\n"
     "            military_time: std::fs::read_to_string(format!(\n"
     "                \"{}/.config/cosmic/com.system76.CosmicAppletTime/v1/military_time\",\n"
     "                std::env::var(\"HOME\").unwrap_or_default()\n"
     "            ))\n"
     "            .is_ok_and(|v| v.trim() == \"true\"),\n        };"),
    # messages
    (r"(enum Message \{\n    TogglePopup,)",
     "enum Message {\n    TogglePopup,\n    MacTick,\n    MacCalPrev,\n    MacCalNext,"),
    # clock tick
    (r"(            activation_token_subscription\(0\)\.map\(Message::Token\),\n        \]\))",
     "            activation_token_subscription(0).map(Message::Token),\n"
     "            cosmic::iced::time::every(std::time::Duration::from_secs(2)).map(|_| Message::MacTick),\n        ])"),
    # update arms
    (r"(    fn update\(&mut self, message: Self::Message\) -> app::Task<Self::Message> \{\n        match message \{\n            Message::TogglePopup => \{\n)",
     """    fn update(&mut self, message: Self::Message) -> app::Task<Self::Message> {
        match message {
            Message::MacTick => {
                let now = jiff::Zoned::now();
                if now.strftime("%Y%m%d%H%M").to_string() != self.now.strftime("%Y%m%d%H%M").to_string() {
                    self.now = now;
                }
                return Task::none();
            }
            Message::MacCalPrev => {
                self.cal_month = self.cal_month.checked_sub(jiff::Span::new().months(1)).unwrap_or(self.cal_month);
                return Task::none();
            }
            Message::MacCalNext => {
                self.cal_month = self.cal_month.checked_add(jiff::Span::new().months(1)).unwrap_or(self.cal_month);
                return Task::none();
            }
            Message::TogglePopup => {
                self.cal_month = self.now.date().first_of_month();
"""),
    # panel button: date + time
    (r"(    fn view\(&self\) -> Element<'_, Message> \{\n        self\.core\n            \.applet\n            \.icon_button\(&self\.icon_name\)\n            \.on_press_down\(Message::TogglePopup\)\n            \.into\(\)\n    \})",
     """    fn view(&self) -> Element<'_, Message> {
        let time = if self.military_time { "%H:%M" } else { "%-I:%M %p" };
        let label = self.now.strftime(&format!("%a %b %-d  {time}")).to_string();
        let mark = if self.config.do_not_disturb {
            "  ☾"
        } else if !self.cards.is_empty() {
            "  •"
        } else {
            ""
        };
        let content = row![
            self.core.applet.text(format!("{label}{mark}")),
            container(space::vertical().height(Length::Fixed(
                (self.core.applet.suggested_size(true).1
                    + 2 * self.core.applet.suggested_padding(true).1) as f32
            )))
        ]
        .align_y(Alignment::Center);
        let button = button::custom(content)
            .padding([0, self.core.applet.suggested_padding(true).0])
            .on_press_down(Message::TogglePopup)
            .class(cosmic::theme::Button::AppletIcon);
        cosmic::widget::autosize::autosize(button, MAC_AUTOSIZE_ID.clone()).into()
    }"""),
    # calendar builder, as an inherent method
    (r"(impl Notifications \{\n)",
     r"""\1    fn mac_calendar(&self) -> Element<'_, Message> {
        let first = self.cal_month;
        let today = self.now.date();
        let header = row![
            text::heading(first.strftime("%B %Y").to_string()).width(Length::Fill),
            button::icon(icon::from_name("go-previous-symbolic")).on_press(Message::MacCalPrev),
            button::icon(icon::from_name("go-next-symbolic")).on_press(Message::MacCalNext),
        ]
        .align_y(Alignment::Center);
        let mut grid = cosmic::widget::grid().width(Length::Fill);
        for d in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] {
            grid = grid.push(container(text::caption(d)).center_x(Length::Fixed(36.0)));
        }
        grid = grid.insert_row();
        let offset = first.weekday().to_monday_zero_offset() as i64;
        let start = first.checked_sub(jiff::Span::new().days(offset)).unwrap_or(first);
        for i in 0..42i64 {
            if i > 0 && i % 7 == 0 {
                grid = grid.insert_row();
            }
            let Ok(day) = start.checked_add(jiff::Span::new().days(i)) else { continue };
            let in_month = day.month() == first.month();
            let label = if in_month { day.day().to_string() } else { String::new() };
            let class = if day == today && in_month {
                cosmic::widget::button::ButtonClass::Suggested
            } else {
                cosmic::widget::button::ButtonClass::Text
            };
            grid = grid.push(
                button::custom(container(text::body(label)).center(Length::Fill))
                    .class(class)
                    .width(Length::Fixed(36.0))
                    .height(Length::Fixed(32.0)),
            );
        }
        Column::new().spacing(8).push(header).push(grid).into()
    }

"""),
    # popup: calendar on top
    (r"(        let content = column!\[do_not_disturb, main_content\])",
     "        let content = column![\n"
     "            padded_control(self.mac_calendar()),\n"
     "            padded_control(divider::horizontal::default()).padding([space_xxs, space_s]),\n"
     "            do_not_disturb,\n"
     "            main_content\n"
     "        ]"),
    # autosize id
    (r"(\nfn text_icon\()",
     "\nstatic MAC_AUTOSIZE_ID: LazyLock<cosmic::widget::Id> =\n"
     "    LazyLock::new(|| cosmic::widget::Id::new(\"mac-clock-autosize\"));\n\\1"),
])

# ---------------------------------------------------------------------------
# macOS Notification Center widgets (calendar, weather, world clock, stocks,
# screen time) below the notifications. The widget code lives in
# patches/mac_widgets.rs and is copied into the crate.
import os, shutil
shutil.copy(os.path.join(os.path.dirname(os.path.abspath(__file__)), "mac_widgets.rs"),
            "cosmic-applet-notifications/src/mac_widgets.rs")

patch("cosmic-applet-notifications/Cargo.toml", [
    (r'(jiff = "0\.2"\n)', r'\1serde_json = "1"\n'),
])

patch(L, [
    (r"(\nstatic MAC_AUTOSIZE_ID)", "\nmod mac_widgets;\n\\1"),
    (r"(    military_time: bool,\n\})", "    military_time: bool,\n    mac_widgets: mac_widgets::Widgets,\n}"),
    (r"(            \.is_ok_and\(\|v\| v\.trim\(\) == \"true\"\),\n        \};)",
     "            .is_ok_and(|v| v.trim() == \"true\"),\n            mac_widgets: Default::default(),\n        };"),
    (r"(    MacCalNext,\n)", "    MacCalNext,\n    MacWidgets(mac_widgets::Msg),\n"),
    (r"(            cosmic::iced::time::every\(std::time::Duration::from_secs\(2\)\)\.map\(\|_\| Message::MacTick\),\n)",
     "\\1            self.mac_widgets.subscription().map(Message::MacWidgets),\n"),
    (r"(            Message::MacCalPrev => \{\n)",
     "            Message::MacWidgets(mac_widgets::Msg::Close) => return self.update(Message::TogglePopup),\n"
     "            Message::MacWidgets(m) => {\n"
     "                return self.mac_widgets.update(m).map(|m| cosmic::Action::App(Message::MacWidgets(m)));\n"
     "            }\n\\1"),
    # popup: notifications (when there are any), then the widgets
    (r"        let content = column!\[\n            padded_control\(self\.mac_calendar\(\)\),\n            padded_control\(divider::horizontal::default\(\)\)\.padding\(\[space_xxs, space_s\]\),\n            do_not_disturb,\n            main_content\n        \]",
     "        let _ = do_not_disturb; // Focus / Do Not Disturb lives in the Control Center\n"
     "        let mut content = cosmic::widget::Column::new();\n"
     "        if !self.cards.is_empty() {\n"
     "            content = content.push(main_content);\n"
     "        }\n"
     "        let content = content\n"
     "            .push(self.mac_widgets.view(&self.now).map(Message::MacWidgets))"),
])
