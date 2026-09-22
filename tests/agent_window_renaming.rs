use openpodium::domain::{
    Agent, AgentId, AgentProgram, CanvasLayout, CanvasPoint, CanvasSize, DomainCommand,
    DomainEvent, Name, Node, NodeId, NodeTarget, Timestamp, Workspace, WorkspaceId,
};
use openpodium::persistence::Journal;

fn add_agent() -> DomainCommand {
    DomainCommand::AddAgentNode {
        agent: Agent::with_program(
            AgentId::new(1),
            Name::new("Codex 9").unwrap(),
            None,
            AgentProgram::Codex,
        ),
        node: Node::new(
            NodeId::new(1),
            NodeTarget::Agent(AgentId::new(1)),
            CanvasPoint::new(0.0, 0.0).unwrap(),
            CanvasSize::new(360.0, 260.0).unwrap(),
        ),
    }
}

fn rename(name: &str) -> DomainCommand {
    DomainCommand::RenameAgent {
        agent_id: AgentId::new(1),
        name: Name::new(name).unwrap(),
    }
}

#[test]
fn renaming_preserves_identity_program_state_and_canvas() {
    let mut workspace = Workspace::new(WorkspaceId::new(1), Name::new("Test").unwrap());
    workspace.execute(add_agent()).unwrap();
    let before = workspace.clone();
    let event = workspace.execute(rename("API implementation")).unwrap();
    let agent = workspace.agent(AgentId::new(1)).unwrap();
    let original = before.agent(AgentId::new(1)).unwrap();
    assert_eq!(agent.name().as_str(), "API implementation");
    assert_eq!(agent.id(), original.id());
    assert_eq!(agent.program(), original.program());
    assert_eq!(agent.state(), original.state());
    assert_eq!(workspace.canvas_layout(), before.canvas_layout());
    let mut replay = before;
    replay.apply(&event).unwrap();
    assert_eq!(replay, workspace);
}

#[test]
fn renamed_agent_survives_snapshot_and_event_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("state.sqlite");
    let mut journal = Journal::open(&database).unwrap();
    let mut workspace = Workspace::new(WorkspaceId::new(1), Name::new("Test").unwrap());
    for command in [add_agent(), rename("API implementation"), rename("Review")] {
        journal
            .execute(&mut workspace, command, Timestamp::from_unix_millis(1))
            .unwrap();
    }
    assert_eq!(journal.recover(workspace.id()).unwrap().unwrap(), workspace);
    drop(journal);
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute("DELETE FROM workspace_snapshots WHERE event_sequence > (SELECT MIN(event_sequence) FROM workspace_snapshots)", []).unwrap();
    drop(connection);
    assert_eq!(
        Journal::open(&database)
            .unwrap()
            .recover(workspace.id())
            .unwrap()
            .unwrap(),
        workspace
    );
}

#[test]
fn stale_rename_is_rejected_without_overwriting_the_current_name() {
    let mut workspace = Workspace::new(WorkspaceId::new(1), Name::new("Test").unwrap());
    workspace.execute(add_agent()).unwrap();
    workspace.execute(rename("Review")).unwrap();
    let before = workspace.clone();
    assert!(
        workspace
            .apply(&DomainEvent::AgentRenamed {
                agent_id: AgentId::new(1),
                from: Name::new("Codex 9").unwrap(),
                to: Name::new("Stale edit").unwrap(),
            })
            .is_err()
    );
    assert_eq!(workspace, before);
}

#[test]
fn deleting_and_restoring_a_window_keeps_its_new_name() {
    let mut workspace = Workspace::new(WorkspaceId::new(1), Name::new("Test").unwrap());
    workspace.execute(add_agent()).unwrap();
    workspace.execute(rename("Review")).unwrap();
    let canvas = workspace.canvas_layout();
    workspace
        .execute(DomainCommand::ReplaceCanvas {
            before: canvas.clone(),
            after: CanvasLayout::default(),
        })
        .unwrap();
    assert!(workspace.execute(rename("Deleted agent")).is_err());
    workspace
        .execute(DomainCommand::ReplaceCanvas {
            before: CanvasLayout::default(),
            after: canvas,
        })
        .unwrap();
    assert_eq!(
        workspace.agent(AgentId::new(1)).unwrap().name().as_str(),
        "Review"
    );
}
