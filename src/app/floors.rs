use iced::widget::{column, row, text};
use iced::{Alignment, Element, Fill, Task};

use super::shell;
use super::ui::{action_grid, button, primary_button, section_label, text_input};
use openpodium::domain::WorkspaceId;
use openpodium::domain::{FloorLifecycle, FloorOwner, NodeTarget};
use openpodium::workspaces::{FloorOperation, FloorResult};

use super::{Message as AppMessage, OpenPodium, WorkspaceManager, now};

mod integration;
mod status;

#[derive(Default)]
pub(super) struct UiState {
    busy: bool,
    name: String,
    branch: String,
    discard: Option<(openpodium::domain::WorkspaceId, u64)>,
    confirmation: String,
    integration: integration::UiState,
    status: status::UiState,
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
    Integration(integration::Message),
    Status(status::Message),
}

pub(super) fn is_busy(state: &OpenPodium) -> bool {
    state.floor_ui.busy || state.floor_ui.integration.busy
}

pub(super) fn tick(state: &mut OpenPodium) -> Task<AppMessage> {
    status::scan(state, false)
}

pub(super) fn workspace_changed(state: &mut OpenPodium) {
    state.floor_ui.integration = integration::UiState::default();
    state.floor_ui.discard = None;
    state.floor_ui.confirmation.clear();
}

pub(super) fn node_severities(
    state: &OpenPodium,
) -> std::collections::BTreeMap<openpodium::domain::NodeId, openpodium::git::CollisionSeverity> {
    status::node_severities(state)
}

pub(super) fn changed_paths(state: &OpenPodium) -> Vec<openpodium::git::ChangedPath> {
    let Some(workspace) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
    else {
        return Vec::new();
    };
    let Some(directory) = workspace.active_directory() else {
        return Vec::new();
    };
    let directory = std::path::Path::new(directory.as_str());
    status::changes_for(&state.floor_ui.status, workspace.id(), directory).to_vec()
}

pub(super) fn update(state: &mut OpenPodium, message: Message) -> Task<AppMessage> {
    match message {
        Message::Integration(message) => return integration::update(state, message),
        Message::Status(message) => return status::update(state, message),
        _ => {}
    }
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
                    return status::scan(state, true);
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
                .and_then(|node| match node.reference()? {
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
        Message::Integration(_) | Message::Status(_) => unreachable!("handled above"),
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
        section_label("Git worktree floors"),
        button(text("Discover / refresh worktrees").size(12))
            .padding([6, 10])
            .on_press(AppMessage::Floor(Message::Refresh))
    ]
    .spacing(8);
    if state.floor_ui.busy {
        content = content.push(
            text("Git operation in progress…")
                .size(12)
                .style(shell::muted_text),
        );
    }
    let Some(workspace) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
    else {
        return content.into();
    };
    let selected = workspace.floors().active;
    let main_severity = status::severity_for_main(&state.floor_ui.status, workspace.id());
    content = content.push(
        button(
            row![
                text("Main floor").size(12).width(Fill),
                text(main_severity.map_or(String::new(), |severity| format!("Git {severity}")))
                    .size(11)
                    .style(shell::muted_text),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding([6, 9])
        .style(shell::navigation_button(selected.is_none()))
        .width(Fill)
        .on_press(AppMessage::Floor(Message::Switch(None))),
    );
    for change in status::changes_for_main(&state.floor_ui.status, workspace.id()) {
        content = content.push(
            text(format!(
                "{} · {}",
                status::change_label(&change.kind),
                change.path
            ))
            .size(11)
            .style(shell::muted_text),
        );
    }
    for (id, floor) in &workspace.floors().entries {
        let severity = status::severity_for_path(
            &state.floor_ui.status,
            workspace.id(),
            std::path::Path::new(floor.directory.as_str()),
        );
        let detail = format!(
            "{} · {} · {:?}{}",
            if floor.managed {
                "managed"
            } else {
                "user-owned"
            },
            if floor.dirty { "dirty" } else { "clean" },
            floor.lifecycle,
            severity.map_or(String::new(), |severity| format!(" · Git {severity}"))
        );
        content = content
            .push(
                button(
                    column![
                        text(floor.name.to_string()).size(12),
                        text(detail).size(11).style(shell::muted_text),
                    ]
                    .spacing(1),
                )
                .padding([6, 9])
                .style(shell::navigation_button(selected == Some(*id)))
                .width(Fill)
                .on_press(AppMessage::Floor(Message::Switch(Some(*id)))),
            )
            .push(
                text(format!(
                    "{} · {}",
                    floor.branch.as_deref().unwrap_or("detached"),
                    floor.directory.as_str()
                ))
                .size(11)
                .style(shell::subtle_text),
            );
        for change in status::changes_for(
            &state.floor_ui.status,
            workspace.id(),
            std::path::Path::new(floor.directory.as_str()),
        ) {
            content = content.push(
                text(format!(
                    "{} · {}",
                    status::change_label(&change.kind),
                    change.path
                ))
                .size(11)
                .style(shell::muted_text),
            );
        }
        if floor.lifecycle == FloorLifecycle::Available {
            content = content.push(
                button(text("Preview integration into main").size(12))
                    .padding([6, 10])
                    .width(Fill)
                    .on_press(AppMessage::Floor(Message::Integration(
                        integration::Message::Preview(*id),
                    ))),
            );
        }
        if floor.managed && floor.lifecycle == FloorLifecycle::Available {
            content = content.push(action_grid([
                button(text("Clean up").size(12))
                    .padding([6, 10])
                    .on_press(AppMessage::Floor(Message::Remove(*id)))
                    .into(),
                button(text("Discard…").size(12))
                    .padding([6, 10])
                    .style(shell::danger_button)
                    .on_press(AppMessage::Floor(Message::RequestDiscard(*id)))
                    .into(),
            ]));
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
        .push(
            text("Select an agent or task node first to record it as the owner.")
                .size(11)
                .style(shell::muted_text),
        )
        .push(
            primary_button(text("Create floor").size(13))
                .on_press(AppMessage::Floor(Message::Create)),
        );
    if let Some((id, floor)) = state.floor_ui.discard
        && id == workspace.id()
        && let Some(floor) = workspace.floors().entries.get(&floor)
    {
        content = content
            .push(text(format!("Discard {} at {}? This deletes its checkout, including uncommitted and untracked files. The branch and canvas history are retained. Type the floor name to confirm.", floor.name, floor.directory.as_str())).size(12))
            .push(text_input("Exact floor name", &state.floor_ui.confirmation).on_input(|s| AppMessage::Floor(Message::Confirmation(s))))
            .push(action_grid([
                button(text("Discard checkout").size(12))
                    .padding([6, 10])
                    .style(shell::danger_button)
                    .on_press(AppMessage::Floor(Message::ConfirmDiscard))
                    .into(),
                button(text("Keep floor").size(12))
                    .padding([6, 10])
                    .on_press(AppMessage::Floor(Message::Keep))
                    .into(),
            ]));
    }
    content
        .push(status::view(state))
        .push(integration::view(state))
        .into()
}
