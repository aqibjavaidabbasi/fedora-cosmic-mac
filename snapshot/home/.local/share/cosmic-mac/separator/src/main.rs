//! A thin vertical divider for the COSMIC dock, like the line in the macOS Dock
//! between apps and the Downloads/Trash stacks. Horizontal on a vertical dock.

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
        let (size, _) = self.core.applet.suggested_size(false);
        let (_, pad) = self.core.applet.suggested_padding(false);
        let long = (size + 2 * pad) as f32 * 0.7;
        let line = container(space::horizontal())
            .class(theme::Container::custom(|t| cosmic::iced::widget::container::Style {
                background: Some(Background::Color({
                    let mut c: Color = t.cosmic().on_bg_color().into();
                    c.a = 0.35;
                    c
                })),
                ..Default::default()
            }));
        let line = if self.core.applet.is_horizontal() {
            line.width(Length::Fixed(1.0)).height(Length::Fixed(long))
        } else {
            line.width(Length::Fixed(long)).height(Length::Fixed(1.0))
        };
        let outer = container(line).padding(if self.core.applet.is_horizontal() { [0, 6] } else { [6, 0] });
        self.core.applet.autosize_window(outer).into()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
