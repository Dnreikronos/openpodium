use std::collections::BTreeMap;

use iced::Element;
use iced::widget::{button, column, row, text};
use openpodium::domain::{TaskId, Workspace, WorkspaceId};
use openpodium::timeline::{
    AttentionLevel, RecoveryAction, TimelineItem, attention_counts, task_summaries,
};

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
        text("Orchestration").size(24),
        text(format!(
            "{} blocked · {} failed",
            counts.blocked, counts.failed
        )),
        button(if selected_task.is_none() {
            "✓ All events"
        } else {
            "All events"
        })
        .on_press(Message::Filter(None)),
        text("Agents").size(18),
    ]
    .spacing(8);
    for agent in workspace.agents() {
        content = content.push(text(format!("{} · {}", agent.name(), agent.state())));
    }
    content = content.push(text("Tasks").size(18));

    for task in task_summaries(workspace) {
        let marker = if task.needs_attention() { "! " } else { "" };
        let label = format!("{marker}{} · {}", task.title(), task.state());
        let mut task_row = row![
            button(text(if selected_task == Some(task.id()) {
                format!("✓ {label}")
            } else {
                label
            }))
            .on_press(Message::Filter(Some(task.id()))),
        ]
        .spacing(6);
        for action in task.actions() {
            let message = match action {
                RecoveryAction::Inspect => Message::Inspect(task.id()),
                RecoveryAction::Retry | RecoveryAction::Cancel | RecoveryAction::Resume => {
                    Message::Recover {
                        task_id: task.id(),
                        action,
                    }
                }
            };
            task_row = task_row.push(button(action_label(action)).on_press(message));
        }
        content = content.push(task_row.wrap());
    }

    content = content.push(text("Timeline").size(18));
    let mut visible = 0_usize;
    for item in items
        .iter()
        .filter(|item| selected_task.is_none_or(|task_id| item.task_id() == Some(task_id)))
    {
        visible += 1;
        let attention = match item.attention() {
            AttentionLevel::Urgent => "! ",
            AttentionLevel::Informational => "✓ ",
            AttentionLevel::None => "",
        };
        let mut event = row![
            column![
                text(format!(
                    "{attention}{} · {}",
                    item.title(),
                    item.occurred_at().as_unix_millis()
                )),
                text(item.detail()).size(12),
            ]
            .spacing(2),
        ]
        .spacing(6);
        if let Some(task_id) = item.task_id() {
            event = event.push(button("Inspect").on_press(Message::Inspect(task_id)));
        }
        content = content.push(event);
    }
    if visible == 0 {
        content = content.push(text("No events for this filter."));
    }
    content.into()
}

fn action_label(action: RecoveryAction) -> &'static str {
    match action {
        RecoveryAction::Inspect => "Inspect",
        RecoveryAction::Retry => "Retry",
        RecoveryAction::Cancel => "Cancel",
        RecoveryAction::Resume => "Resume",
    }
}
