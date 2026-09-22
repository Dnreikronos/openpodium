use iced::widget::{column, row, text};
use iced::{Element, Fill, Task};
use openpodium::domain::WorkspaceId;

use super::{Controls, Message, OpenPodium, active_workspace_id, floors, portals, shell, ui};

pub(super) fn confirmation(state: &OpenPodium, id: WorkspaceId) -> Element<'_, Message> {
    let name = state
        .workspaces
        .as_ref()
        .and_then(|manager| manager.workspace(id))
        .map_or("Workspace", |workspace| workspace.name());
    let busy = floors::is_busy(state);
    column![
        text(format!("Remove “{name}” from OpenPodium?")).size(15),
        text("Its running terminals will stop. Your project folder and files will not be deleted.")
            .size(13),
        text("Open the same folder again to restore this workspace and its saved board.")
            .size(12)
            .style(shell::muted_text),
        busy.then(|| text(
            "Wait for the current workspace operation to finish before removing it."
        )
        .size(12)),
        row![
            iced::widget::Space::new().width(Fill),
            ui::button("Cancel").on_press(Message::CloseControls),
            ui::button("Remove workspace")
                .style(shell::danger_button)
                .on_press_maybe((!busy).then_some(Message::RemoveWorkspace(id))),
        ]
        .spacing(8),
    ]
    .spacing(12)
    .into()
}

pub(super) fn remove(state: &mut OpenPodium, id: WorkspaceId) -> Task<Message> {
    if state.controls != Some(Controls::RemoveWorkspace(id)) || floors::is_busy(state) {
        return Task::none();
    }
    let was_active = active_workspace_id(state) == Some(id);
    state.flush_terminal_transcripts();
    let result = state
        .workspaces
        .as_mut()
        .ok_or_else(|| "Workspace storage is unavailable".to_owned())
        .and_then(|manager| {
            manager
                .remove_workspace(id)
                .map_err(|error| error.to_string())
        });
    if let Err(error) = result {
        state.notice = Some(error);
        return Task::none();
    }
    state.terminals.retain(|key, _| key.workspace_id != id);
    if let Some(ipc) = &state.ipc {
        ipc.remove_workspace(id.get());
    }
    state.timeline_items.remove(&id);
    state.timeline_high_watermarks.remove(&id);
    state.supervisor_collisions.remove(&id);
    state.navigation_ui.index.remove_workspace(id);
    state.navigation_ui.indexed_revisions.remove(&id);
    state.navigation_ui.stale.remove(&id);
    state.navigation_ui.search_generation = state.navigation_ui.search_generation.wrapping_add(1);
    state
        .navigation_ui
        .search_results
        .retain(|result| result.document.target.workspace_id != id);
    if was_active {
        floors::workspace_changed(state);
        state.reset_canvas_session();
        state.load_active_settings();
        state.restore_terminal_transcripts();
    }
    state.controls = None;
    state.notice = None;
    state.sync_ipc_directory();
    super::refresh_supervisor_snapshot(state);
    portals::tick(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Session, TerminalKey, now, terminal};
    use openpodium::domain::{
        Agent, AgentId, CanvasPoint, CanvasSize, DomainCommand, Name, Node, NodeId, NodeTarget,
    };
    use openpodium::workspaces::WorkspaceManager;
    use std::collections::BTreeMap;

    #[test]
    fn removal_requires_confirmation_and_cleans_only_the_target_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let mut manager = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
        let first = manager.create_workspace(temp.path(), now()).unwrap();
        let second = manager.create_workspace(temp.path(), now()).unwrap();
        for id in [first, second] {
            manager
                .execute(
                    id,
                    DomainCommand::AddAgentNode {
                        agent: Agent::new(AgentId::new(1), Name::new("Shell").unwrap(), None),
                        node: Node::new(
                            NodeId::new(1),
                            NodeTarget::Agent(AgentId::new(1)),
                            CanvasPoint::new(0.0, 0.0).unwrap(),
                            CanvasSize::new(360.0, 260.0).unwrap(),
                        ),
                    },
                    now(),
                )
                .unwrap();
        }
        let key = |workspace_id| TerminalKey {
            workspace_id,
            node_id: NodeId::new(1),
        };
        let sessions = [first, second]
            .into_iter()
            .map(|id| {
                (
                    key(id),
                    Session::starting(terminal::GridSize::for_node(360.0, 260.0), 7),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut state = crate::app::tests::test_state(manager, sessions);
        state.ipc = Some(openpodium::ipc::IpcService::start(temp.path()).unwrap());
        state.sync_ipc_directory();
        let _ = remove(&mut state, second);
        assert!(
            state
                .workspaces
                .as_ref()
                .unwrap()
                .workspace(second)
                .is_some()
        );
        let _ = crate::app::update(
            &mut state,
            Message::OpenControls(Controls::RemoveWorkspace(second)),
        );
        let _ = crate::app::update(&mut state, Message::CloseControls);
        let _ = remove(&mut state, second);
        assert!(
            state
                .workspaces
                .as_ref()
                .unwrap()
                .workspace(second)
                .is_some()
        );
        let _ = crate::app::update(
            &mut state,
            Message::OpenControls(Controls::RemoveWorkspace(second)),
        );
        let _ = remove(&mut state, second);
        assert_eq!(active_workspace_id(&state), Some(first));
        assert!(!state.terminals.contains_key(&key(second)));
        assert!(state.terminals[&key(first)].is_active());
        assert!(
            state
                .ipc
                .as_ref()
                .unwrap()
                .connection_info(second.get(), 1)
                .is_none()
        );
        assert!(
            state
                .ipc
                .as_ref()
                .unwrap()
                .connection_info(first.get(), 1)
                .is_some()
        );
        assert!(state.controls.is_none());
        let _ = super::super::navigation::update(
            &mut state,
            crate::navigation_panel::Message::IndexBuilt {
                workspace_id: second,
                revision: None,
                documents: Ok(vec![]),
            },
        );
        assert!(!state.navigation_ui.indexed_revisions.contains_key(&second));
    }
}
