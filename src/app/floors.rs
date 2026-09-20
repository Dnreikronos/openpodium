use iced::widget::{button, column, row, text, text_input};
use iced::{Element, Task};
use openpodium::domain::WorkspaceId;
use openpodium::domain::{FloorLifecycle, FloorOwner, NodeTarget};
use openpodium::workspaces::{FloorOperation, FloorResult};

use super::{Message as AppMessage, OpenPodium, WorkspaceManager, now};

#[derive(Default)]
pub(super) struct UiState {
    busy: bool,
    name: String,
    branch: String,
    discard: Option<(openpodium::domain::WorkspaceId, u64)>,
    confirmation: String,
}

#[derive(Clone)]
pub(super) enum Message {
    Completed(WorkspaceId, Result<FloorResult, String>),
    Name(String),
    Branch(String),
    Refresh,
    Create,
    Switch(Option<u64>),
    Remove(u64),
    RequestDiscard(u64),
    Confirmation(String),
    ConfirmDiscard,
    Keep,
}

pub(super) fn is_busy(state: &OpenPodium) -> bool {
    state.floor_ui.busy
}

pub(super) fn update(state: &mut OpenPodium, message: Message) -> Task<AppMessage> {
    match message {
        Message::Name(name) => {
            state.floor_ui.name = name;
            return Task::none();
        }
        Message::Branch(branch) => {
            state.floor_ui.branch = branch;
            return Task::none();
        }
        Message::Confirmation(value) => {
            state.floor_ui.confirmation = value;
            return Task::none();
        }
        Message::Keep => {
            state.floor_ui.discard = None;
            state.floor_ui.confirmation.clear();
            return Task::none();
        }
        Message::Completed(id, result) => {
            state.floor_ui.busy = false;
            let Some(manager) = state.workspaces.as_mut() else {
                return Task::none();
            };
            let previous = manager.workspace(id).and_then(|w| w.floors().active);
            match result.and_then(|result| manager.finish_floor_operation(result, now())) {
                Ok(()) => {
                    let active = manager.workspace(id).and_then(|w| w.floors().active);
                    let visible = manager.active_workspace_id() == Some(id);
                    state.floor_ui.discard = None;
                    state.floor_ui.confirmation.clear();
                    state.notice = None;
                    if visible && active != previous {
                        state.reset_canvas_session();
                    }
                }
                Err(error) => state.notice = Some(error),
            }
            return Task::none();
        }
        _ => {}
    }
    if state.floor_ui.busy {
        return Task::none();
    }
    let Some(manager) = state.workspaces.as_mut() else {
        return Task::none();
    };
    let Some(id) = manager.active_workspace_id() else {
        return Task::none();
    };
    let operation = match message {
        Message::Refresh => FloorOperation::Refresh,
        Message::Create => {
            let owner = state
                .canvas_selection
                .first()
                .and_then(|node| manager.workspace(id)?.node(*node))
                .and_then(|node| match node.target() {
                    NodeTarget::Agent(id) => Some(FloorOwner::Agent(id)),
                    NodeTarget::Task(id) => Some(FloorOwner::Task(id)),
                    _ => None,
                });
            FloorOperation::Create {
                name: state.floor_ui.name.clone(),
                branch: state.floor_ui.branch.clone(),
                owner,
            }
        }
        Message::Switch(floor) => {
            match manager.switch_floor(id, floor, now()) {
                Ok(()) => {
                    state.reset_canvas_session();
                    state.notice = None;
                }
                Err(error) => state.notice = Some(error),
            }
            return Task::none();
        }
        Message::Remove(floor) => FloorOperation::Remove {
            floor,
            discard: false,
        },
        Message::RequestDiscard(floor) => {
            state.floor_ui.discard = Some((id, floor));
            state.floor_ui.confirmation.clear();
            return Task::none();
        }
        Message::ConfirmDiscard => {
            let Some((workspace, floor)) = state.floor_ui.discard else {
                return Task::none();
            };
            if workspace != id {
                state.notice =
                    Some("Switch back to the floor's workspace before confirming".to_owned());
                return Task::none();
            }
            let Some(target) = manager
                .workspace(id)
                .and_then(|w| w.floors().entries.get(&floor))
            else {
                return Task::none();
            };
            if state.floor_ui.confirmation != target.name.as_str() {
                state.notice =
                    Some("Type the floor name to confirm discarding its checkout".to_owned());
                return Task::none();
            }
            FloorOperation::Remove {
                floor,
                discard: true,
            }
        }
        _ => return Task::none(),
    };
    if let FloorOperation::Remove { floor, .. } = operation {
        let workspace = manager.workspace(id).expect("active workspace exists");
        if state.terminals.iter().any(|(key, session)| {
            key.workspace_id == id
                && workspace.floors().node_floors.get(&key.node_id) == Some(&floor)
                && session.is_active()
        }) {
            state.notice = Some("Stop all terminals on this floor before cleanup".to_owned());
            return Task::none();
        }
    }
    match manager.prepare_floor_operation(id, operation) {
        Ok(job) => {
            state.floor_ui.busy = true;
            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || job.run())
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| r)
                },
                move |result| AppMessage::Floor(Message::Completed(id, result)),
            )
        }
        Err(error) => {
            state.notice = Some(error);
            Task::none()
        }
    }
}

pub(super) fn view(state: &OpenPodium) -> Element<'_, AppMessage> {
    let mut content = column![
        text("Git worktree floors").size(18),
        button("Discover / refresh worktrees").on_press(AppMessage::Floor(Message::Refresh))
    ]
    .spacing(8);
    if state.floor_ui.busy {
        content = content.push(text("Git operation in progress…"));
    }
    let Some(workspace) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
    else {
        return content.into();
    };
    let selected = workspace.floors().active;
    content = content.push(
        button(if selected.is_none() {
            "Main floor (selected)"
        } else {
            "Main floor"
        })
        .on_press(AppMessage::Floor(Message::Switch(None))),
    );
    for (id, floor) in &workspace.floors().entries {
        let label = format!(
            "{}{} · {} · {} · {:?}",
            floor.name,
            if selected == Some(*id) {
                " (selected)"
            } else {
                ""
            },
            if floor.managed {
                "managed"
            } else {
                "user-owned"
            },
            if floor.dirty { "dirty" } else { "clean" },
            floor.lifecycle
        );
        content = content
            .push(button(text(label)).on_press(AppMessage::Floor(Message::Switch(Some(*id)))))
            .push(
                text(format!(
                    "{} · {}",
                    floor.branch.as_deref().unwrap_or("detached"),
                    floor.directory.as_str()
                ))
                .size(12),
            );
        if floor.managed && floor.lifecycle == FloorLifecycle::Available {
            content = content.push(
                row![
                    button("Clean up").on_press(AppMessage::Floor(Message::Remove(*id))),
                    button("Discard checkout…")
                        .on_press(AppMessage::Floor(Message::RequestDiscard(*id)))
                ]
                .spacing(8),
            );
        }
    }
    content = content
        .push(
            text_input("Floor name", &state.floor_ui.name)
                .on_input(|s| AppMessage::Floor(Message::Name(s))),
        )
        .push(
            text_input("New branch", &state.floor_ui.branch)
                .on_input(|s| AppMessage::Floor(Message::Branch(s))),
        )
        .push(text("Select an agent or task node first to record it as the owner.").size(12))
        .push(button("Create floor").on_press(AppMessage::Floor(Message::Create)));
    if let Some((id, floor)) = state.floor_ui.discard
        && id == workspace.id()
        && let Some(floor) = workspace.floors().entries.get(&floor)
    {
        content = content.push(text(format!("Discard {} at {}? This deletes its checkout, including uncommitted and untracked files. The branch and canvas history are retained. Type the floor name to confirm.", floor.name, floor.directory.as_str())))
            .push(text_input("Exact floor name", &state.floor_ui.confirmation).on_input(|s| AppMessage::Floor(Message::Confirmation(s))))
            .push(row![button("Discard checkout").on_press(AppMessage::Floor(Message::ConfirmDiscard)), button("Keep floor").on_press(AppMessage::Floor(Message::Keep))].spacing(8));
    }
    content.into()
}
