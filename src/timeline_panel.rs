use std::collections::BTreeMap;

use iced::widget::{column, row, text};
use iced::{Alignment, Element, Fill};
use openpodium::domain::{TaskId, Workspace, WorkspaceId};
use openpodium::timeline::{RecoveryAction, task_summaries};

use crate::app::shell;
use crate::app::ui::{action_grid, button, count_badge, section};

#[derive(Debug, Clone)]
pub enum Message {
    Filter(Option<TaskId>),
    Inspect(TaskId),
    Recover {
        task_id: TaskId,
        action: RecoveryAction,
    },
}

#[derive(Default)]
pub struct UiState {
    selected_tasks: BTreeMap<WorkspaceId, TaskId>,
}

impl UiState {
    pub fn selected_task(&self, workspace_id: WorkspaceId) -> Option<TaskId> {
        self.selected_tasks.get(&workspace_id).copied()
    }

    pub fn select_task(&mut self, workspace_id: WorkspaceId, task_id: Option<TaskId>) {
        if let Some(task_id) = task_id {
            self.selected_tasks.insert(workspace_id, task_id);
        } else {
            self.selected_tasks.remove(&workspace_id);
        }
    }
}

pub fn panel<'a>(workspace: &'a Workspace, state: &'a UiState) -> Element<'a, Message> {
    let selected_task = state.selected_task(workspace.id());
    let mut content = column![].spacing(8);
    if selected_task.is_some() {
        content =
            content.push(button(text("Clear selection").size(12)).on_press(Message::Filter(None)));
    }
    for task in task_summaries(workspace) {
        let selected = selected_task == Some(task.id());
        let mut heading = row![
            text(task.title().to_string()).size(12).width(Fill),
            text(task.state().to_string())
                .size(11)
                .style(shell::muted_text),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        if task.needs_attention() {
            heading = heading.push(count_badge(1, true));
        }
        let actions = task.actions();
        content = content.push(
            button(heading)
                .padding([6, 9])
                .style(shell::navigation_button(selected))
                .width(Fill)
                .on_press(Message::Filter(Some(task.id()))),
        );
        if selected && !actions.is_empty() {
            content = content.push(action_grid(actions.into_iter().map(|action| {
                let message = match action {
                    RecoveryAction::Inspect => Message::Inspect(task.id()),
                    RecoveryAction::Retry | RecoveryAction::Cancel | RecoveryAction::Resume => {
                        Message::Recover {
                            task_id: task.id(),
                            action,
                        }
                    }
                };
                button(text(action_label(action)).size(12))
                    .padding([6, 10])
                    .on_press(message)
                    .into()
            })));
        }
    }
    if task_summaries(workspace).is_empty() {
        content = content.push(
            text("No tasks in this workspace.")
                .size(13)
                .style(shell::muted_text),
        );
    }
    section("Task actions", content)
}

fn action_label(action: RecoveryAction) -> &'static str {
    match action {
        RecoveryAction::Inspect => "Inspect",
        RecoveryAction::Retry => "Retry",
        RecoveryAction::Cancel => "Cancel",
        RecoveryAction::Resume => "Resume",
    }
}
