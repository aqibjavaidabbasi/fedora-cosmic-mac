//! A thin divider for the COSMIC dock, like the line in the macOS Dock between
//! apps and the Downloads/Trash stacks, drawn on its slice of the glass shelf.

use cosmic::{
    Element, Task, app,
    iced::{Background, Color, Length},
    theme,
    widget::{container, space},
};

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<Separator>(())
}

#[derive(Default)]
struct Separator {
    core: app::Core,
}

impl cosmic::Application for Separator {
    type Message = ();
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    const APP_ID: &'static str = "com.aqib.DockSeparator";

    fn init(core: app::Core, _flags: ()) -> (Self, app::Task<()>) {
        (Self { core }, Task::none())
    }

    fn core(&self) -> &app::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut app::Core {
        &mut self.core
    }

    fn update(&mut self, _message: ()) -> app::Task<()> {
        Task::none()
    }

    fn view(&self) -> Element<'_, ()> {
        // A 1px line on its own slice of the macOS glass shelf (see patches/dock.py):
        // the shelf is the bottom (icon + 16)px of the dock, where icon is 80% of the
        // panel's icon size; above it the dock is transparent.
        let (full, _) = self.core.applet.suggested_size(false);
        let (_, cross) = self.core.applet.suggested_padding(false);
        let total = (full + 2 * cross) as f32;
        let shelf_h = ((full as f32) * 0.8).round() + 16.0;
        let line = container(space::horizontal())
            .width(Length::Fixed(1.0))
            .height(Length::Fixed(shelf_h * 0.6))
            .class(theme::Container::custom(|t| cosmic::iced::widget::container::Style {
                background: Some(Background::Color({
                    let mut c: Color = t.cosmic().on_bg_color().into();
                    c.a = 0.35;
                    c
                })),
                ..Default::default()
            }));
        let shelf = container(line)
            .width(Length::Fill)
            .height(Length::Fixed(shelf_h))
            .center_x(Length::Fill)
            .center_y(Length::Fixed(shelf_h))
            .class(theme::Container::custom(|t| {
                let mut bg: Color = t.cosmic().background(false).base.into();
                bg.a = 0.55;
                cosmic::iced::widget::container::Style { background: Some(Background::Color(bg)), ..Default::default() }
            }));
        let slice = cosmic::widget::column::with_children(vec![space::vertical().height(Length::Fill).into(), shelf.into()])
            .width(Length::Fixed(13.0))
            .height(Length::Fixed(total));
        self.core.applet.autosize_window(slice).into()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
