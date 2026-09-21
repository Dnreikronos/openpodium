use std::collections::BTreeMap;

use iced::widget::{column, row, text};
use iced::{Alignment, Element, Fill};
use openpodium::domain::{TaskId, Workspace, WorkspaceId};
use openpodium::timeline::{
    AttentionLevel, RecoveryAction, TimelineItem, attention_counts, task_summaries,
};

use crate::app::shell;
use crate::app::ui::{action_grid, button, count_badge, section, section_label};

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

pub fn panel<'a>(
    workspace: &'a Workspace,
    items: &'a [TimelineItem],
    state: &'a UiState,
) -> Element<'a, Message> {
    let workspace_id = workspace.id();
    let selected_task = state.selected_task(workspace_id);
    let counts = attention_counts(workspace);
    let mut content = column![
        row![
            text("blocked").size(11).style(shell::muted_text),
            count_badge(counts.blocked, true),
            text("failed").size(11).style(shell::muted_text),
            count_badge(counts.failed, true),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
        button(text("All events").size(12))
            .padding([5, 12])
            .style(shell::navigation_button(selected_task.is_none()))
            .width(Fill)
            .on_press(Message::Filter(None)),
        section_label("Agents"),
    ]
    .spacing(8);
    for agent in workspace.agents() {
        content = content.push(
            row![
                text(agent.name().to_string()).size(12).width(Fill),
                text(agent.state().to_string())
                    .size(11)
                    .style(shell::muted_text),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        );
    }
    content = content.push(section_label("Tasks"));

    // The selected task carries the filter highlight, and its recovery
    // actions only appear for that task instead of on every row.
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

    content = content.push(section_label("Timeline"));
    let mut visible = 0_usize;
    for item in items
        .iter()
        .filter(|item| selected_task.is_none_or(|task_id| item.task_id() == Some(task_id)))
    {
        visible += 1;
        let attention = match item.attention() {
            AttentionLevel::Urgent => "!",
            AttentionLevel::Informational => "✓",
            AttentionLevel::None => "·",
        };
        let mut event = row![
            text(attention).size(12).style(shell::muted_text),
            column![
                text(item.title().to_string()).size(12),
                text(item.detail()).size(11).style(shell::muted_text),
            ]
            .spacing(1)
            .width(Fill),
        ]
        .spacing(8);
        if let Some(task_id) = item.task_id() {
            event = event.push(
                button(text("Inspect").size(12))
                    .padding([5, 10])
                    .on_press(Message::Inspect(task_id)),
            );
        }
        content = content.push(event);
    }
    if visible == 0 {
        content = content.push(
            text("No events for this filter.")
                .size(12)
                .style(shell::muted_text),
        );
    }
    section("Orchestration", content)
}

fn action_label(action: RecoveryAction) -> &'static str {
    match action {
        RecoveryAction::Inspect => "Inspect",
        RecoveryAction::Retry => "Retry",
        RecoveryAction::Cancel => "Cancel",
        RecoveryAction::Resume => "Resume",
    }
}
