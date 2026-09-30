// macOS Launchpad view for the COSMIC App Library (copied into src/ by patches/app-library.py).
// Full screen over the blurred wallpaper: a search pill on top, a 7 x 5 page of big icons
// with white names, folders (app groups) as glass tiles with mini icons, and page dots.
// Scroll or swipe to change pages; click empty space or press Escape to close.

use super::*;
use cosmic::iced::widget::column as col;

pub(super) const COLS: usize = 7;
pub(super) const ROWS: usize = 5;
pub(super) const PER_PAGE: usize = COLS * ROWS;

enum Item {
    App(usize),
    Folder(usize),
    NewFolder,
}

fn items(app: &CosmicAppLibrary) -> Vec<Item> {
    // ids and icons are cleared while hidden; only show entries that have both
    let n = app.entry_path_input.len().min(app.entry_ids.len()).min(app.entry_icon_handles.len());
    let mut v: Vec<Item> = (0..n).map(Item::App).collect();
    if app.cur_group.is_none() && app.search_value.is_empty() {
        v.extend((0..app.config.groups.len()).map(Item::Folder));
        v.push(Item::NewFolder);
    }
    v
}

pub(super) fn pages(app: &CosmicAppLibrary) -> usize {
    items(app).len().div_ceil(PER_PAGE).max(1)
}

fn glass(radius: f32, alpha: f32) -> theme::Container<'static> {
    theme::Container::Custom(Box::new(move |_| container::Style {
        background: Some(Color::from_rgba(1., 1., 1., alpha).into()),
        border: Border { radius: radius.into(), width: 1.0, color: Color::from_rgba(1., 1., 1., 0.18) },
        ..Default::default()
    }))
}

fn tile<'a>(art: Element<'a, Message>, name: String) -> Element<'a, Message> {
    col![
        art,
        container(text(name).size(13).width(Length::Shrink).align_x(cosmic::iced::widget::text::Alignment::Center))
            .align_x(Alignment::Center)
            .width(Length::Fill)
            .height(Length::Fixed(40.)),
    ]
    .spacing(4)
    .height(Length::Fixed(136.))
    .align_x(Alignment::Center)
    .width(Length::Fill)
    .into()
}

fn folder_art<'a>(app: &'a CosmicAppLibrary, g: usize) -> Element<'a, Message> {
    let apps = app.config.filtered(Some(g), "", &app.all_entries);
    let minis: Vec<Element<'a, Message>> = apps
        .iter()
        .take(9)
        .map(|e| e.icon.as_cosmic_icon().icon().width(Length::Fixed(22.)).height(Length::Fixed(22.)).into())
        .collect();
    let rows: Vec<Element<'a, Message>> = minis
        .into_iter()
        .chunks(3)
        .into_iter()
        .map(|r| row(r.collect::<Vec<_>>()).spacing(4).into())
        .collect();
    let inner: Element<'a, Message> = if rows.is_empty() {
        icon::icon(from_name(app.config.groups[g].icon.clone()).into())
            .width(Length::Fixed(40.))
            .height(Length::Fixed(40.))
            .into()
    } else {
        col(rows).spacing(4).into()
    };
    container(inner)
        .width(Length::Fixed(88.))
        .height(Length::Fixed(88.))
        .padding(10)
        .align_x(Alignment::Center)
        .align_y(Vertical::Center)
        .class(glass(20., 0.22))
        .into()
}

pub(super) fn view<'a>(app: &'a CosmicAppLibrary) -> Element<'a, Message> {
    let white = Color::WHITE;
    let all = items(app);
    let n_pages = all.len().div_ceil(PER_PAGE).max(1);
    let page = app.mac_page.min(n_pages - 1);

    // --- top: search pill, or the open folder's name ---------------------------------
    let top: Element<'a, Message> = if let Some(g) = app.cur_group {
        let name: Element<'a, Message> = if let Some(edit_name) = app.edit_name.as_ref() {
            text_input(app.config.groups[g].name(), edit_name)
                .on_input(Message::EditName)
                .on_paste(Message::EditName)
                .on_clear(Message::EditName(String::new()))
                .on_submit(|_| Message::SubmitName)
                .id(EDIT_GROUP_ID.clone())
                .width(Length::Fixed(260.0))
                .size(16)
                .into()
        } else {
            text(app.config.groups[g].name()).size(28).into()
        };
        let icon_btn = |name: &'static str, msg: Option<Message>| {
            button::custom(icon::icon(from_name(name).into()).width(Length::Fixed(20.)).height(Length::Fixed(20.)))
                .padding(8)
                .class(Button::Icon)
                .on_press_maybe(msg)
        };
        row![
            icon_btn("go-previous-symbolic", Some(Message::SelectGroup(None))),
            name,
            icon_btn("edit-symbolic", app.edit_name.is_none().then(|| Message::StartEditName(app.config.groups[g].name()))),
            icon_btn("edit-delete-symbolic", Some(Message::Delete(g))),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
    } else {
        search_input("Search Applications", app.search_value.as_str())
            .on_input(Message::InputChanged)
            .on_paste(Message::InputChanged)
            .on_submit(|_| Message::StartCurAppFocus)
            .style(TextInput::Search)
            .width(Length::Fixed(300.0))
            .size(14)
            .id(SEARCH_ID.clone())
            .into()
    };

    // --- one page of the grid ---------------------------------------------------------
    let cells: Vec<Element<'a, Message>> = all
        .iter()
        .skip(page * PER_PAGE)
        .take(PER_PAGE)
        .map(|item| -> Element<'a, Message> {
            match *item {
                Item::App(i) => {
                    let entry = &app.entry_path_input[i];
                    let gpu_idx = app.gpus.as_ref().map(|gpus| {
                        if entry.prefers_dgpu {
                            gpus.iter().position(|gpu| !gpu.default).unwrap_or(0)
                        } else {
                            gpus.iter().position(|gpu| gpu.default).unwrap_or(0)
                        }
                    });
                    let dup = entry.path.as_ref().and_then(|path| app.duplicates.get(path));
                    let selected = app.menu.is_some_and(|m| m == i);
                    ApplicationButton::new(
                        app.entry_ids[i].clone(),
                        &entry.name,
                        app.entry_icon_handles[i].clone(),
                        &entry.path,
                        move |rect| Message::OpenContextMenu(rect, i),
                        if app.menu.is_none() {
                            Some(Message::ActivateApp(i, gpu_idx))
                        } else if selected {
                            Some(Message::CloseContextMenu)
                        } else {
                            None
                        },
                        dup,
                        selected,
                        app.menu.is_none().then_some(Message::StartDrag(i)),
                        app.menu.is_none().then_some(Message::FinishDrag(false)),
                        app.menu.is_none().then_some(Message::CancelDrag),
                    )
                    .into()
                }
                Item::Folder(g) => {
                    let group_ref = Some(g);
                    let offered = app.offer_group == Some(group_ref);
                    dnd_destination_for_data::<AppletString, Message>(
                        button::custom(tile(folder_art(app, g), app.config.groups[g].name()))
                            .class(if offered { Button::Standard } else { Button::IconVertical })
                            .padding(theme::spacing().space_s)
                            .width(Length::Fill)
                            .on_press_maybe(app.menu.is_none().then_some(Message::SelectGroup(group_ref))),
                        move |data, _| {
                            Message::FinishDndOffer(group_ref, data.and_then(|data| load_desktop_file(&[], data.0)))
                        },
                    )
                    .drag_id(g as u64 + 1)
                    .on_enter(move |_, _, _| Message::StartDndOffer(group_ref))
                    .on_leave(move || Message::LeaveDndOffer(group_ref))
                    .into()
                }
                Item::NewFolder => {
                    let art = container(
                        icon::icon(from_name("list-add-symbolic").into())
                            .width(Length::Fixed(32.))
                            .height(Length::Fixed(32.)),
                    )
                    .width(Length::Fixed(88.))
                    .height(Length::Fixed(88.))
                    .align_x(Alignment::Center)
                    .align_y(Vertical::Center)
                    .class(glass(20., 0.10));
                    button::custom(tile(art.into(), "New Folder".to_string()))
                        .class(Button::IconVertical)
                        .padding(theme::spacing().space_s)
                        .width(Length::Fill)
                        .on_press(Message::StartNewGroup)
                        .into()
                }
            }
        })
        .collect();
    let mut cells = cells.into_iter().peekable();
    let mut grid_rows: Vec<Element<'a, Message>> = Vec::new();
    while cells.peek().is_some() {
        let mut r: Vec<Element<'a, Message>> = cells.by_ref().take(COLS).collect();
        let missing = COLS - r.len();
        if missing > 0 {
            r.push(space::horizontal().width(Length::FillPortion(missing as u16)).into());
        }
        grid_rows.push(container(row(r).spacing(8)).height(Length::Fill).align_y(Vertical::Top).into());
    }
    // a full page always has ROWS rows, so rows sit at the same height on every page
    while grid_rows.len() < ROWS {
        grid_rows.push(space::vertical().height(Length::Fill).into());
    }
    let grid = container(col(grid_rows).width(Length::Fill).height(Length::Fill))
        .max_width((app.size.width * 0.72).clamp(900., 1500.))
        .max_height(ROWS as f32 * 190.)
        .center_x(Length::Fill)
        .height(Length::Fill);

    // --- page dots -----------------------------------------------------------------------
    let dots: Element<'a, Message> = if n_pages > 1 {
        row((0..n_pages).map(|p| {
            let on = p == page;
            button::custom(space::horizontal().width(Length::Fixed(8.)))
                .width(Length::Fixed(8.))
                .height(Length::Fixed(8.))
                .padding(0)
                .class(Button::Custom {
                    active: Box::new(move |_, _| dot(on)),
                    disabled: Box::new(move |_| dot(on)),
                    hovered: Box::new(move |_, _| dot(true)),
                    pressed: Box::new(move |_, _| dot(true)),
                })
                .on_press(Message::MacPage(p))
                .into()
        }))
        .spacing(10)
        .into()
    } else {
        space::vertical().height(Length::Fixed(8.)).into()
    };

    let content = col![
        space::vertical().height(Length::Fixed(app.margin + 28.)),
        top,
        space::vertical().height(Length::Fixed(44.)),
        grid,
        dots,
        // keep clear of the Dock
        space::vertical().height(Length::Fixed(112.)),
    ]
    .align_x(Alignment::Center)
    .width(Length::Fill)
    .height(Length::Fill);

    let backdrop = mouse_area(
        container(space::horizontal().width(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .class(theme::Container::Custom(Box::new(|_| container::Style {
                background: Some(Color::from_rgba(0.02, 0.02, 0.08, 0.30).into()),
                ..Default::default()
            }))),
    )
    .on_press(if app.menu.is_some() { Message::CloseContextMenu } else { Message::Hide });

    container(stack![backdrop, content].width(Length::Fill).height(Length::Fill))
        .class(theme::Container::Custom(Box::new(move |_| container::Style {
            text_color: Some(white),
            icon_color: Some(white),
            ..Default::default()
        })))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn dot(on: bool) -> button::Style {
    button::Style {
        background: Some(Color::from_rgba(1., 1., 1., if on { 0.95 } else { 0.35 }).into()),
        border_radius: 4.0.into(),
        ..button::Style::new()
    }
}
