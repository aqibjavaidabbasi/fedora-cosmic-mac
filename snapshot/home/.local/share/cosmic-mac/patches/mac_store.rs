//! macOS App Store look for the COSMIC Store (cosmic-mac patches/store.py).
//!
//! The window is drawn like the System Settings patch (patches/mac_sidebar.rs): traffic
//! lights and a frosted, blurred sidebar (search on top, colored rounded icons, grouped,
//! soft selection) beside an opaque content area. COSMIC's header bar is hidden.
//! Discover (the Explore page) is laid out like the Mac App Store: a large featured
//! banner, three editorial cards, then shelves of apps with GET / OPEN buttons.

use cosmic::{
    Element,
    iced::{Alignment, Background, Border, Color, Gradient, Length, Radians, gradient},
    theme,
    widget::{self, button, column, container, icon, row, text},
};

use crate::app_id::AppId;
use crate::app_info::AppInfo;
use crate::backend::BackendName;
use crate::explore::ExplorePage;
use crate::icon_cache::icon_cache_handle;
use crate::nav::NavPage;
use crate::operation::OperationKind;
use crate::search::SearchResult;
use crate::{App, Message};

/// (page, Mac label, icon background)
type Entry = (NavPage, &'static str, u32);

const GROUPS: &[&[Entry]] = &[
    &[(NavPage::Explore, "Discover", 0x0A84FF)],
    &[
        (NavPage::Create, "Create", 0xFF9F0A),
        (NavPage::Work, "Work", 0x5E5CE6),
        (NavPage::Develop, "Develop", 0x8E8E93),
        (NavPage::Learn, "Learn", 0x34C759),
        (NavPage::Game, "Play", 0xFF375F),
        (NavPage::Relax, "Relax", 0xBF5AF2),
        (NavPage::Socialize, "Socialize", 0x32ADE6),
        (NavPage::Utilities, "Utilities", 0x8E8E93),
        (NavPage::Applets, "Applets", 0x30B0C7),
    ],
    &[(NavPage::Installed, "Installed", 0x0A84FF), (NavPage::Updates, "Updates", 0x34C759)],
];

/// When the blur was last requested. libcosmic turns window blur off again whenever the
/// theme says "no frosted windows", so it is re-requested at most once a second.
static LAST_BLUR: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// Ask the compositor to blur behind the window (the see-through sidebar), like Settings.
pub fn blur_task(app: &App) -> cosmic::Task<cosmic::Action<Message>> {
    let Some(id) = app.core.main_window_id() else {
        return cosmic::Task::none();
    };
    let mut last = LAST_BLUR.lock().unwrap();
    if last.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(1)) {
        return cosmic::Task::none();
    }
    *last = Some(std::time::Instant::now());
    cosmic::iced::runtime::task::effect(cosmic::iced::runtime::Action::PlatformSpecific(
        cosmic::iced::runtime::platform_specific::Action::Wayland(
            cosmic::iced::runtime::platform_specific::wayland::Action::BlurSurface(
                id,
                Some(vec![cosmic::iced::Rectangle { x: 0., y: 0., width: f32::MAX, height: f32::MAX }]),
            ),
        ),
    ))
}

const MAC_BLUE: Color = Color::from_rgb(0.039, 0.518, 1.0); // #0A84FF

fn rgb(hex: u32) -> Color {
    Color::from_rgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn glyph(page: NavPage) -> &'static str {
    match page {
        NavPage::Explore => "store-home-symbolic",
        NavPage::Create => "store-create-symbolic",
        NavPage::Work => "store-work-symbolic",
        NavPage::Develop => "store-develop-symbolic",
        NavPage::Learn => "store-learn-symbolic",
        NavPage::Game => "store-game-symbolic",
        NavPage::Relax => "store-relax-symbolic",
        NavPage::Socialize => "store-socialize-symbolic",
        NavPage::Utilities => "store-utilities-symbolic",
        NavPage::Applets => "store-applets-symbolic",
        NavPage::Installed => "store-installed-symbolic",
        NavPage::Updates => "store-updates-symbolic",
    }
}

/// White glyph on a 28px rounded square, like macOS sidebar icons (same as Settings).
fn squircle<'a>(page: NavPage, hex: u32) -> Element<'a, Message> {
    let bg = rgb(hex);
    let mut handle = icon_cache_handle(glyph(page), 16); // bundled at 16px only
    handle.symbolic = true; // draw the glyph in the white icon color
    container(icon::icon(handle).size(17))
        .width(Length::Fixed(28.0))
        .height(Length::Fixed(28.0))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .class(theme::Container::custom(move |_| container::Style {
            background: Some(Background::Color(bg)),
            border: Border { radius: 7.0.into(), ..Default::default() },
            icon_color: Some(Color::WHITE),
            text_color: Some(Color::WHITE),
            ..Default::default()
        }))
        .into()
}

fn row_style(selected: bool) -> theme::Button {
    let make = move |hover: bool, pressed: bool| {
        move |_focused: bool, t: &cosmic::Theme| {
            let c = t.cosmic();
            let on: Color = c.on_bg_color().into();
            let bg = if selected {
                Some(Background::Color(Color { a: if c.is_dark { 0.16 } else { 0.10 }, ..on }))
            } else if pressed {
                Some(Background::Color(Color { a: 0.12, ..on }))
            } else if hover {
                Some(Background::Color(Color { a: 0.07, ..on }))
            } else {
                None
            };
            button::Style {
                background: bg,
                border_radius: 7.0.into(),
                text_color: Some(on),
                icon_color: Some(on),
                ..button::Style::new()
            }
        }
    };
    theme::Button::Custom {
        active: Box::new(make(false, false)),
        hovered: Box::new(make(true, false)),
        pressed: Box::new(make(false, true)),
        disabled: Box::new(move |t| make(false, false)(false, t)),
    }
}

fn traffic(hex: u32, dim: f32) -> button::Style {
    let c = rgb(hex);
    button::Style {
        background: Some(Background::Color(Color { r: c.r * dim, g: c.g * dim, b: c.b * dim, a: 1.0 })),
        border_radius: 6.0.into(),
        border_width: 0.5,
        border_color: Color::from_rgba(0.0, 0.0, 0.0, 0.2),
        ..button::Style::new()
    }
}

/// The sidebar: traffic lights, search, then the grouped pages.
fn sidebar(app: &App) -> Element<'_, Message> {
    let active = app.nav_model.active_data::<NavPage>().copied();

    let search = widget::text_input::search_input("Search", &app.search_input)
        .id(app.search_id.clone())
        .on_input(Message::SearchInput)
        .on_clear(Message::SearchClear)
        .on_submit(Message::SearchSubmit)
        .width(Length::Fill);

    let mut list = column::with_capacity(20).spacing(1);
    for (gi, group) in GROUPS.iter().enumerate() {
        if gi > 0 {
            list = list.push(widget::Space::new().height(Length::Fixed(12.0)));
        }
        for (page, label, hex) in group.iter() {
            let Some(entity) = app.nav_model.iter().find(|e| app.nav_model.data::<NavPage>(*e) == Some(page)) else {
                continue;
            };
            let mut line = row![squircle(*page, *hex), text::body(*label).size(14).width(Length::Fill)]
                .spacing(11)
                .align_y(Alignment::Center);
            if *page == NavPage::Updates {
                if let Some(n) = app.updates.as_ref().map(Vec::len).filter(|n| *n > 0) {
                    line = line.push(
                        container(text::caption(n.to_string()).size(11))
                            .padding([1, 7])
                            .class(theme::Container::custom(|_| container::Style {
                                background: Some(Background::Color(MAC_BLUE)),
                                border: Border { radius: 9.0.into(), ..Default::default() },
                                text_color: Some(Color::WHITE),
                                ..Default::default()
                            })),
                    );
                }
            }
            let selected = active == Some(*page) && app.search_results.is_none();
            list = list.push(
                button::custom(line)
                    .width(Length::Fill)
                    .padding([5, 8])
                    .class(row_style(selected))
                    .on_press(Message::MacNav(entity)),
            );
        }
    }

    let light = |hex: u32, msg: Message| {
        button::custom(widget::Space::new().width(Length::Fixed(12.0)).height(Length::Fixed(12.0)))
            .padding(0)
            .class(theme::Button::Custom {
                active: Box::new(move |_, _| traffic(hex, 1.0)),
                hovered: Box::new(move |_, _| traffic(hex, 0.8)),
                pressed: Box::new(move |_, _| traffic(hex, 0.6)),
                disabled: Box::new(move |_| traffic(hex, 1.0)),
            })
            .on_press(msg)
    };
    let lights = widget::mouse_area(
        container(row![light(0xFF5F57, Message::MacClose), light(0xFEBC2E, Message::MacMinimize), light(0x28C840, Message::MacMaximize)].spacing(8))
            .width(Length::Fill)
            .padding([14, 16, 10, 16]),
    )
    .on_press(Message::MacDrag)
    .on_double_click(Message::MacMaximize);

    column![
        lights,
        container(column![search, widget::scrollable(list).height(Length::Fill)].spacing(12)).padding([0, 10, 10, 10])
    ]
    .width(Length::Fixed(250.0))
    .height(Length::Fill)
    .into()
}

/// The whole window: frosted sidebar beside an opaque content area whose top strip
/// drags the window (with the back button, busy spinner and repositories menu).
pub fn window<'a>(app: &'a App, content: Element<'a, Message>) -> Element<'a, Message> {
    let busy = app.fetching_backends
        || app.update_apps_scheduled
        || app.update_apps_in_progress
        || app.explore_results_handle.is_some()
        || !app.pending_backend_updates.is_empty()
        || !app.pending_operations.is_empty()
        || !app.waiting_installed.is_empty()
        || !app.waiting_updates.is_empty();
    let mut bar = row::with_capacity(4).spacing(8).align_y(Alignment::Center);
    bar = bar.push_maybe(app.back_button());
    bar = bar.push(widget::space::horizontal());
    if busy {
        bar = bar.push(widget::indeterminate_circular().size(18.0));
    }
    bar = bar.push(
        widget::tooltip(
            button::icon(icon::from_name("application-menu-symbolic"))
                .on_press(Message::ToggleContextPage(crate::ContextPage::Repositories)),
            text(crate::fl!("manage-repositories")),
            widget::tooltip::Position::Bottom,
        ),
    );
    let toolbar = widget::mouse_area(container(bar).padding([8, 14, 4, 14]).width(Length::Fill).height(Length::Fixed(46.0)))
        .on_press(Message::MacDrag)
        .on_double_click(Message::MacMaximize);

    let content = container(column![toolbar, content].height(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill)
        .class(theme::Container::custom(|t| {
            let c = t.cosmic();
            container::Style {
                background: Some(Background::Color(c.background(false).base.into())),
                border: Border { radius: [0.0, 12.0, 12.0, 0.0].into(), ..Default::default() },
                ..Default::default()
            }
        }));
    let side = container(sidebar(app)).height(Length::Fill).class(theme::Container::custom(|t| {
        let c = t.cosmic();
        // same frosted tint as the System Settings sidebar
        let tint = if c.is_dark { Color::from_rgba(0.15, 0.15, 0.16, 0.94) } else { Color::from_rgba(0.925, 0.925, 0.945, 0.94) };
        container::Style {
            background: Some(Background::Color(tint)),
            border: Border { radius: [12.0, 0.0, 0.0, 12.0].into(), ..Default::default() },
            ..Default::default()
        }
    }));
    let divider = container(widget::Space::new().width(Length::Fixed(1.0)))
        .height(Length::Fill)
        .class(theme::Container::custom(|t| {
            let on: Color = t.cosmic().on_bg_color().into();
            container::Style { background: Some(Background::Color(Color { a: 0.08, ..on })), ..Default::default() }
        }));
    row![side, divider, content].height(Length::Fill).into()
}

// ---------------------------------------------------------------------------
// Discover
// ---------------------------------------------------------------------------

fn dim_text(t: &cosmic::Theme) -> cosmic::iced::widget::text::Style {
    let mut c: Color = t.cosmic().on_bg_color().into();
    c.a = 0.55;
    cosmic::iced::widget::text::Style { color: Some(c), ..Default::default() }
}

fn secondary<'a>(s: impl Into<String>) -> Element<'a, Message> {
    text::caption(s.into()).class(theme::Text::Custom(dim_text)).into()
}

fn eyebrow<'a>(s: impl Into<String>) -> Element<'a, Message> {
    text::caption(s.into().to_uppercase()).size(11).class(theme::Text::Custom(dim_text)).into()
}

/// Gray capsule with bold blue text: GET, OPEN, or progress while installing.
fn get_button<'a>(app: &'a App, backend_name: BackendName, id: &AppId, info: &std::sync::Arc<AppInfo>) -> Element<'a, Message> {
    for (_id, (op, progress)) in app.pending_operations.iter() {
        if op.backend_name == backend_name && op.package_ids.iter().any(|p| p == id) {
            return container(widget::determinate_linear(*progress).width(Length::Fixed(56.0)).girth(Length::Fixed(4.0)))
                .width(Length::Fixed(64.0))
                .center_x(Length::Fixed(64.0))
                .into();
        }
    }
    let installed = app.is_installed(backend_name, id, info);
    let (label, msg) = if installed {
        match info.desktop_ids.first() {
            Some(desktop_id) => ("OPEN", Some(Message::OpenDesktopId(desktop_id.clone()))),
            None => ("INSTALLED", None),
        }
    } else {
        ("GET", Some(Message::Operation(OperationKind::Install, backend_name, id.clone(), info.clone())))
    };
    let style = move |hover: f32, t: &cosmic::Theme| {
        let c = t.cosmic();
        let on: Color = c.on_bg_color().into();
        button::Style {
            background: Some(Background::Color(Color { a: if c.is_dark { 0.14 } else { 0.07 } + hover, ..on })),
            border_radius: 999.0.into(),
            text_color: Some(MAC_BLUE),
            ..button::Style::new()
        }
    };
    button::custom(text::body(label).size(12).font(cosmic::font::bold()))
        .padding([3, 14])
        .class(theme::Button::Custom {
            active: Box::new(move |_, t| style(0.0, t)),
            hovered: Box::new(move |_, t| style(0.06, t)),
            pressed: Box::new(move |_, t| style(0.12, t)),
            disabled: Box::new(move |t| style(0.0, t)),
        })
        .on_press_maybe(msg)
        .into()
}

fn app_icon<'a>(result: &'a SearchResult, size: u16) -> Element<'a, Message> {
    match &result.icon_opt {
        Some(handle) => icon::icon(handle.clone()).size(size).into(),
        None => widget::Space::new().width(Length::Fixed(size as f32)).height(Length::Fixed(size as f32)).into(),
    }
}

fn card_style(t: &cosmic::Theme) -> container::Style {
    let c = t.cosmic();
    let on: Color = c.on_bg_color().into();
    container::Style {
        background: Some(Background::Color(Color { a: if c.is_dark { 0.06 } else { 0.035 }, ..on })),
        border: Border { radius: 14.0.into(), width: 1.0, color: Color { a: 0.06, ..on } },
        ..Default::default()
    }
}

/// The big banner at the top of Discover.
fn hero<'a>(result: &'a SearchResult, page: ExplorePage) -> Element<'a, Message> {
    let banner = row![
        column![
            text::caption("EDITORS' CHOICE").size(11).class(theme::Text::Custom(|_| cosmic::iced::widget::text::Style {
                color: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.75)), ..Default::default()
            })),
            text::title1(&result.info.name).class(theme::Text::Custom(|_| cosmic::iced::widget::text::Style { color: Some(Color::WHITE), ..Default::default() })),
            text::body(&result.info.summary).class(theme::Text::Custom(|_| cosmic::iced::widget::text::Style {
                color: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.85)), ..Default::default()
            })),
        ]
        .spacing(6)
        .width(Length::Fill),
        app_icon(result, 128),
    ]
    .align_y(Alignment::Center)
    .spacing(24);
    widget::mouse_area(
        container(banner)
            .padding([28, 32])
            .width(Length::Fill)
            .height(Length::Fixed(230.0))
            .align_y(Alignment::Center)
            .class(theme::Container::custom(|_| container::Style {
                background: Some(Background::Gradient(Gradient::Linear(
                    gradient::Linear::new(Radians(2.2))
                        .add_stop(0.0, Color::from_rgb8(0x0A, 0x5C, 0xFF))
                        .add_stop(0.55, Color::from_rgb8(0x5E, 0x5C, 0xE6))
                        .add_stop(1.0, Color::from_rgb8(0xBF, 0x5A, 0xF2)),
                ))),
                border: Border { radius: 16.0.into(), ..Default::default() },
                ..Default::default()
            })),
    )
    .on_press(Message::SelectExploreResult(page, 0))
    .into()
}

/// One of the three editorial cards under the banner.
fn editorial<'a>(result: &'a SearchResult, page: ExplorePage, i: usize, eyebrow_text: &str) -> Element<'a, Message> {
    let body = row![
        column![
            eyebrow(eyebrow_text),
            text::title4(&result.info.name),
            secondary(result.info.summary.clone()),
        ]
        .spacing(4)
        .width(Length::Fill),
        container(app_icon(result, 72))
            .width(Length::Fixed(96.0))
            .height(Length::Fixed(96.0))
            .center(Length::Fixed(96.0))
            .class(theme::Container::custom(|t| {
                let on: Color = t.cosmic().on_bg_color().into();
                container::Style {
                    background: Some(Background::Color(Color { a: 0.06, ..on })),
                    border: Border { radius: 48.0.into(), ..Default::default() },
                    ..Default::default()
                }
            })),
    ]
    .align_y(Alignment::Center)
    .spacing(12);
    widget::mouse_area(
        container(body)
            .padding([16, 18])
            .width(Length::Fill)
            .height(Length::Fixed(150.0))
            .align_y(Alignment::Center)
            .class(theme::Container::custom(card_style)),
    )
    .on_press(Message::SelectExploreResult(page, i))
    .into()
}

/// A compact app row: icon, name and summary, GET button, hairline below.
fn shelf_item<'a>(app: &'a App, result: &'a SearchResult, page: ExplorePage, i: usize) -> Element<'a, Message> {
    let line = row![
        app_icon(result, 44),
        column![
            text::body(&result.info.name).font(cosmic::font::semibold()).wrapping(cosmic::iced::widget::text::Wrapping::None),
            text::caption(&result.info.summary)
                .class(theme::Text::Custom(dim_text))
                .wrapping(cosmic::iced::widget::text::Wrapping::None),
        ]
        .spacing(2)
        .width(Length::Fill)
        .clip(true),
        get_button(app, result.backend_name, &result.id, &result.info),
    ]
    .align_y(Alignment::Center)
    .spacing(12);
    let hairline = container(widget::Space::new().height(Length::Fixed(1.0)))
        .width(Length::Fill)
        .class(theme::Container::custom(|t| {
            let on: Color = t.cosmic().on_bg_color().into();
            container::Style { background: Some(Background::Color(Color { a: 0.08, ..on })), ..Default::default() }
        }));
    widget::mouse_area(column![container(line).padding([8, 4]), container(hairline).padding([0, 0, 0, 60])].width(Length::Fill))
        .on_press(Message::SelectExploreResult(page, i))
        .into()
}

fn section_header<'a>(title: String, page: ExplorePage) -> Element<'a, Message> {
    row![
        text::title3(title).width(Length::Fill),
        button::custom(text::body("See All").class(theme::Text::Custom(|_| cosmic::iced::widget::text::Style { color: Some(MAC_BLUE), ..Default::default() })))
            .padding([4, 6])
            .class(theme::Button::Text)
            .on_press(Message::ExplorePage(Some(page))),
    ]
    .align_y(Alignment::Center)
    .into()
}

/// The Discover page, like the Mac App Store's.
pub fn discover(app: &App, width: f32) -> Element<'_, Message> {
    let mut page_col = column::with_capacity(24).spacing(22).padding([4, 28, 32, 28]).width(Length::Fill);

    // featured banner + three editorial cards from Editors' Choice (else Popular)
    let (feature_page, featured) = [ExplorePage::EditorsChoice, ExplorePage::PopularApps]
        .iter()
        .find_map(|p| app.explore_results.get(p).filter(|r| !r.is_empty()).map(|r| (*p, r)))
        .map_or((ExplorePage::EditorsChoice, &[][..]), |(p, r)| (p, r.as_slice()));
    if let Some(first) = featured.first() {
        page_col = page_col.push(hero(first, feature_page));
    }
    let eyebrows = ["Get started", "From the editors", "Great new app"];
    let cards: Vec<Element<'_, Message>> = featured
        .iter()
        .enumerate()
        .skip(1)
        .take(3)
        .map(|(i, r)| editorial(r, feature_page, i, eyebrows[(i - 1) % 3]))
        .collect();
    if !cards.is_empty() {
        page_col = page_col.push(row(cards).spacing(14));
    }

    // shelves: title + See All, then rows of apps in columns
    let cols = ((width - 56.0) / 330.0).floor().clamp(1.0, 4.0) as usize;
    for &page in ExplorePage::all() {
        if page == feature_page {
            continue;
        }
        let Some(results) = app.explore_results.get(&page).filter(|r| !r.is_empty()) else {
            continue;
        };
        let shown = results.len().min(cols * 3);
        let mut grid = row::with_capacity(cols).spacing(24);
        for c in 0..cols {
            let mut col = column::with_capacity(3).width(Length::Fill);
            for r in 0..3 {
                let i = c * 3 + r;
                if i < shown {
                    col = col.push(shelf_item(app, &results[i], page, i));
                }
            }
            grid = grid.push(col);
        }
        page_col = page_col.push(column![section_header(page.title(), page), grid].spacing(6));
    }
    page_col.into()
}
