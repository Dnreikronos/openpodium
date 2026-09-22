use iced::widget::{column, row, text};
use iced::{Element, Fill, Task};
use openpodium::domain::{AgentId, DomainCommand, Name, NodeId, NodeTarget, WorkspaceId};

use super::{Controls, Message as AppMessage, OpenPodium, active_workspace_id, now, shell, ui};

const INPUT_ID: &str = "agent-window-name";

#[derive(Default)]
pub(super) struct State {
    target: Option<(WorkspaceId, AgentId, Name)>,
    draft: String,
    error: Option<String>,
}

#[derive(Clone)]
pub(super) enum Message {
    Open(NodeId),
    NameChanged(String),
    Save,
}

pub(super) fn update(state: &mut OpenPodium, message: Message) -> Task<AppMessage> {
    match message {
        Message::Open(node_id) => {
            let Some(workspace) = state
                .workspaces
                .as_ref()
                .and_then(|manager| manager.active_workspace())
            else {
                return Task::none();
            };
            let Some(NodeTarget::Agent(agent_id)) =
                workspace.node(node_id).and_then(|node| node.reference())
            else {
                return Task::none();
            };
            let Some(agent) = workspace.agent(agent_id) else {
                return Task::none();
            };
            state.rename_ui = State {
                target: Some((workspace.id(), agent_id, agent.name().clone())),
                draft: agent.name().as_str().to_owned(),
                error: None,
            };
            state.cancel_connection();
            state.focused_terminal = None;
            state.focused_portal = None;
            state.canvas_selection = vec![node_id];
            state.controls = Some(Controls::Rename);
            return iced::widget::operation::focus(INPUT_ID);
        }
        Message::NameChanged(value) => {
            state.rename_ui.draft = value;
            state.rename_ui.error = None;
        }
        Message::Save => {
            let Some((workspace_id, agent_id, original)) = state.rename_ui.target.clone() else {
                return Task::none();
            };
            if state.controls != Some(Controls::Rename)
                || active_workspace_id(state) != Some(workspace_id)
            {
                return Task::none();
            }
            let name = match Name::new(&state.rename_ui.draft) {
                Ok(name) => name,
                Err(error) => {
                    state.rename_ui.error = Some(error.to_string());
                    return Task::none();
                }
            };
            let workspace = state
                .workspaces
                .as_ref()
                .and_then(|manager| manager.workspace(workspace_id));
            if workspace
                .and_then(|workspace| workspace.agent(agent_id))
                .map(|agent| agent.name())
                != Some(&original)
            {
                state.rename_ui.error = Some(
                    "This agent changed or was deleted. Close and reopen the name editor."
                        .to_owned(),
                );
                return Task::none();
            }
            if name == original {
                state.controls = None;
                return Task::none();
            }
            let result = state
                .workspaces
                .as_mut()
                .expect("workspace was checked above")
                .execute(
                    workspace_id,
                    DomainCommand::RenameAgent { agent_id, name },
                    now(),
                );
            match result {
                Ok(_) => {
                    state.controls = None;
                    state.rename_ui = State::default();
                    state.canvas_revision = state.canvas_revision.wrapping_add(1);
                    state.sync_ipc_directory();
                }
                Err(error) => state.rename_ui.error = Some(error.to_string()),
            }
        }
    }
    Task::none()
}

pub(super) fn view(state: &State) -> Element<'_, AppMessage> {
    column![
        text("Window name").size(12).style(shell::muted_text),
        ui::text_input("Name", &state.draft)
            .id(INPUT_ID)
            .on_input(|name| AppMessage::Rename(Message::NameChanged(name)))
            .on_submit(AppMessage::Rename(Message::Save)),
        text("Changes the agent name without restarting its terminal.")
            .size(12)
            .style(shell::muted_text),
        state.error.as_ref().map(|error| text(error).size(12)),
        row![
            iced::widget::Space::new().width(Fill),
            ui::button("Cancel").on_press(AppMessage::CloseControls),
            ui::primary_button("Save name").on_press(AppMessage::Rename(Message::Save)),
        ]
        .spacing(8),
    ]
    .spacing(10)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Session, TerminalKey, terminal};
    use openpodium::domain::{Agent, CanvasPoint, CanvasSize, Node, Timestamp};
    use openpodium::workspaces::WorkspaceManager;
    use std::collections::BTreeMap;

    fn fixture() -> (tempfile::TempDir, OpenPodium, TerminalKey) {
        let temp = tempfile::tempdir().unwrap();
        let mut manager = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
        let workspace_id = manager
            .create_workspace(temp.path(), Timestamp::from_unix_millis(1))
            .unwrap();
        let node_id = NodeId::new(1);
        manager
            .execute(
                workspace_id,
                DomainCommand::AddAgentNode {
                    agent: Agent::new(AgentId::new(1), Name::new("Codex 9").unwrap(), None),
                    node: Node::new(
                        node_id,
                        NodeTarget::Agent(AgentId::new(1)),
                        CanvasPoint::new(0.0, 0.0).unwrap(),
                        CanvasSize::new(360.0, 260.0).unwrap(),
                    ),
                },
                Timestamp::from_unix_millis(2),
            )
            .unwrap();
        let key = TerminalKey {
            workspace_id,
            node_id,
        };
        let session = Session::starting(terminal::GridSize::for_node(360.0, 260.0), 17);
        let state = crate::app::tests::test_state(manager, BTreeMap::from([(key, session)]));
        (temp, state, key)
    }

    fn name(state: &OpenPodium) -> &str {
        state
            .workspaces
            .as_ref()
            .unwrap()
            .active_workspace()
            .unwrap()
            .agent(AgentId::new(1))
            .unwrap()
            .name()
            .as_str()
    }

    #[test]
    fn renaming_saves_without_replacing_the_terminal_session() {
        let (temp, mut state, key) = fixture();
        let _ = update(&mut state, Message::Open(key.node_id));
        assert_eq!(state.rename_ui.draft, "Codex 9");
        let _ = update(
            &mut state,
            Message::NameChanged("  API implementation  ".to_owned()),
        );
        let _ = update(&mut state, Message::Save);
        assert_eq!(name(&state), "API implementation");
        assert_eq!(state.controls, None);
        assert_eq!(state.terminals.len(), 1);
        assert_eq!(state.terminals[&key].generation(), 17);
        assert!(state.terminals[&key].is_active());
        drop(state);
        let restored = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
        assert_eq!(
            restored
                .workspace(key.workspace_id)
                .unwrap()
                .agent(AgentId::new(1))
                .unwrap()
                .name()
                .as_str(),
            "API implementation"
        );
    }

    #[test]
    fn cancel_discards_the_draft_and_late_save_is_ignored() {
        let (_temp, mut state, key) = fixture();
        let _ = update(&mut state, Message::Open(key.node_id));
        let _ = update(&mut state, Message::NameChanged("Unsaved".to_owned()));
        let _ = crate::app::update(&mut state, AppMessage::CloseControls);
        let _ = update(&mut state, Message::Save);
        assert_eq!(name(&state), "Codex 9");
        let _ = update(&mut state, Message::Open(key.node_id));
        assert_eq!(state.rename_ui.draft, "Codex 9");
    }

    #[test]
    fn invalid_or_conflicting_names_leave_the_editor_open() {
        let (_temp, mut state, key) = fixture();
        let _ = update(&mut state, Message::Open(key.node_id));
        for invalid in ["   ".to_owned(), "é".repeat(Name::MAX_CHARS + 1)] {
            let _ = update(&mut state, Message::NameChanged(invalid));
            let _ = update(&mut state, Message::Save);
            assert_eq!(name(&state), "Codex 9");
            assert_eq!(state.controls, Some(Controls::Rename));
            assert!(state.rename_ui.error.is_some());
        }
        state
            .workspaces
            .as_mut()
            .unwrap()
            .execute(
                key.workspace_id,
                DomainCommand::RenameAgent {
                    agent_id: AgentId::new(1),
                    name: Name::new("Concurrent rename").unwrap(),
                },
                now(),
            )
            .unwrap();
        let _ = update(&mut state, Message::NameChanged("Stale edit".to_owned()));
        let _ = update(&mut state, Message::Save);
        assert_eq!(name(&state), "Concurrent rename");
        assert_eq!(state.controls, Some(Controls::Rename));
        assert!(state.rename_ui.error.as_ref().unwrap().contains("changed"));
    }
}
