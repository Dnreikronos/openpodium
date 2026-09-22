use iced::widget::{Column, column, text};
use iced::{Fill, Task};
use openpodium::domain::{AgentProgram, NodeId, NodeTarget};
use openpodium::runtime::ProcessSpec;

use super::{Controls, Message, OpenPodium, Session, active_terminal_key, button};

pub(super) fn needs_recovery(state: &OpenPodium, node: NodeId) -> bool {
    active_terminal_key(state, node)
        .is_some_and(|key| !state.terminals.get(&key).is_some_and(Session::is_active))
}

pub(super) fn open(state: &mut OpenPodium, node: NodeId) -> Task<Message> {
    if program(state, node).is_some() && needs_recovery(state, node) {
        state.cancel_connection();
        state.canvas_selection = vec![node];
        state.focused_terminal = None;
        state.focused_portal = None;
        state.notice = None;
        state.controls = Some(Controls::TerminalRecovery(node));
    }
    Task::none()
}

fn program(state: &OpenPodium, node: NodeId) -> Option<AgentProgram> {
    let workspace = state.workspaces.as_ref()?.active_workspace()?;
    let NodeTarget::Agent(agent) = workspace.node(node)?.reference()? else {
        return None;
    };
    Some(workspace.agent(agent)?.program())
}

pub(super) fn can_resume(original: AgentProgram, resumed: AgentProgram) -> bool {
    matches!(resumed, AgentProgram::Codex | AgentProgram::Claude)
        && (original == AgentProgram::Shell || original == resumed)
}

pub(super) fn resume_spec(spec: ProcessSpec, resumed: AgentProgram) -> ProcessSpec {
    match resumed {
        AgentProgram::Codex => spec.arg("resume"),
        AgentProgram::Claude => spec.arg("--resume"),
        _ => spec,
    }
}

pub(super) fn view(state: &OpenPodium, node: NodeId) -> Column<'_, Message> {
    let Some(program) = program(state, node) else {
        return column![text("This terminal window is no longer available.")];
    };
    let mut content = column![
        text("This terminal has stopped. What you see on the canvas is saved output.").size(14),
        text("Start a new terminal, or choose a saved agent conversation to resume. Nothing runs until you choose.").size(13),
        button(text(if program == AgentProgram::Shell { "Start a new shell".to_owned() } else { format!("Start new {} session", program.label()) }).size(13))
            .on_press(Message::StartTerminal(node)).width(Fill),
    ].spacing(12);
    for resumed in [AgentProgram::Codex, AgentProgram::Claude] {
        if can_resume(program, resumed) {
            content = content.push(
                button(text(format!("Resume {} conversation…", resumed.label())).size(13))
                    .on_press(Message::ResumeTerminal {
                        node_id: node,
                        program: resumed,
                    })
                    .width(Fill),
            );
        }
    }
    if program == AgentProgram::Shell {
        content = content.push(text("A new shell uses this window's project directory. Old shell variables and running jobs cannot be recovered.").size(12));
    }
    if let Some(notice) = &state.notice {
        content = content.push(text(notice).size(12));
    }
    content
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;

    #[test]
    fn stopped_input_and_paste_offer_recovery_without_starting_a_process() {
        let temp = tempfile::tempdir().unwrap();
        let mut manager = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
        let workspace_id = manager.create_workspace(temp.path(), now()).unwrap();
        let node_id = NodeId::new(1);
        manager
            .execute(
                workspace_id,
                DomainCommand::AddAgentNode {
                    agent: Agent::with_program(
                        AgentId::new(1),
                        Name::new("Shell").unwrap(),
                        None,
                        AgentProgram::Shell,
                    ),
                    node: Node::new(
                        node_id,
                        NodeTarget::Agent(AgentId::new(1)),
                        CanvasPoint::new(0.0, 0.0).unwrap(),
                        CanvasSize::new(400.0, 300.0).unwrap(),
                    ),
                },
                now(),
            )
            .unwrap();
        let key = TerminalKey {
            workspace_id,
            node_id,
        };
        let session = Session::restored(
            terminal::GridSize::for_node(400.0, 300.0),
            3,
            b"saved output".to_vec(),
        );
        let before = session.view().cells;
        let mut state = super::super::tests::test_state(manager, BTreeMap::from([(key, session)]));
        for message in [
            canvas::Message::TerminalInput {
                node_id,
                bytes: b"0".to_vec(),
            },
            canvas::Message::TerminalPasteRequested(node_id),
        ] {
            state.controls = None;
            let _ = handle_canvas_message(&mut state, message);
            assert_eq!(state.controls, Some(Controls::TerminalRecovery(node_id)));
            assert!(!state.terminals[&key].is_active());
            assert_eq!(state.terminals[&key].view().cells, before);
        }
        state
            .terminals
            .get_mut(&key)
            .unwrap()
            .prepare_restart(terminal::GridSize::for_node(400.0, 300.0), 4);
        assert!(!needs_recovery(&state, node_id));
    }

    #[test]
    fn resume_uses_a_picker_without_replaying_commands_or_selecting_the_last_session() {
        for (program, executable, argument) in [
            (AgentProgram::Codex, "codex", "resume"),
            (AgentProgram::Claude, "claude", "--resume"),
        ] {
            let spec = resume_spec(
                ProcessSpec::new(executable, "/project").arg("--existing-option"),
                program,
            );
            assert_eq!(spec.working_directory(), std::path::Path::new("/project"));
            assert_eq!(
                spec.arguments(),
                ["--existing-option", argument].map(std::ffi::OsString::from)
            );
            assert!(can_resume(AgentProgram::Shell, program));
            assert!(can_resume(program, program));
        }
        assert!(!can_resume(AgentProgram::Claude, AgentProgram::Codex));
        assert!(!can_resume(AgentProgram::Shell, AgentProgram::Shell));
    }
}
