use iced::widget::{column, container, row, text};
use iced::{Element, Fill, Theme};

use crate::canvas::Camera;
use crate::domain::WorkspaceSummary;

const APP_NAME: &str = "OpenPodium";

type Message = ();

#[derive(Debug, Default)]
struct OpenPodium {
    camera: Camera,
    workspace: WorkspaceSummary,
}

pub(crate) fn run() -> iced::Result {
    iced::application(OpenPodium::default, update, view)
        .title(APP_NAME)
        .theme(Theme::Dark)
        .centered()
        .run()
}

fn update(_state: &mut OpenPodium, _message: Message) {}

fn view(state: &OpenPodium) -> Element<'_, Message> {
    let sidebar = container(
        column![
            text(APP_NAME).size(24),
            text("Workspaces").size(14),
            text(state.workspace.name()),
            text(format!("{} active agents", state.workspace.agent_count())).size(12),
        ]
        .spacing(16),
    )
    .width(240)
    .height(Fill)
    .padding(24);

    let stage = container(
        column![
            text("Your canvas starts here").size(28),
            text("Agent terminals, tasks, and project context will share this space."),
            text(format!("Zoom: {}%", state.camera.zoom_percent())).size(12),
        ]
        .spacing(12),
    )
    .width(Fill)
    .height(Fill)
    .center(Fill);

    row![sidebar, stage].into()
}
