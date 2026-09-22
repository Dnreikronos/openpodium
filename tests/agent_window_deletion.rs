use openpodium::domain::{
    Agent, AgentId, AgentState, CanvasLayout, CanvasPoint, CanvasSize, ChatDraft, ChatMessageId,
    ChatThread, ChatThreadId, DomainCommand, DomainEvent, Name, Node, NodeId, NodeTarget,
    Timestamp, Workspace, WorkspaceId,
};
use openpodium::persistence::Journal;
use openpodium::workspaces::WorkspaceManager;

#[test]
fn a_window_on_another_floor_keeps_the_agent_active() {
    use openpodium::domain::{Floor, FloorLifecycle, WorkspaceDirectory};
    let temp = tempfile::tempdir().unwrap();
    let mut workspace = workspace();
    workspace.execute(DomainCommand::AddNode(node(2))).unwrap();
    let before = workspace.floors().clone();
    let mut after = before.clone();
    let directory = WorkspaceDirectory::new(temp.path().to_str().unwrap()).unwrap();
    after.entries.insert(
        1,
        Floor {
            name: Name::new("Other floor").unwrap(),
            directory: directory.clone(),
            repository: directory,
            branch: None,
            base_revision: "test".to_owned(),
            base_branch: None,
            managed: false,
            ownership_token: None,
            owner: None,
            dirty: false,
            lifecycle: FloorLifecycle::Available,
        },
    );
    after.node_floors.insert(NodeId::new(2), 1);
    workspace
        .execute(DomainCommand::ReplaceFloors { before, after })
        .unwrap();
    workspace
        .execute(DomainCommand::ReplaceCanvas {
            before: workspace.canvas_layout(),
            after: CanvasLayout::default(),
        })
        .unwrap();
    assert!(workspace.canvas_layout().nodes().is_empty());
    assert_eq!(workspace.agent_count(), 1);
    assert_eq!(workspace.all_canvas_layout().nodes().len(), 1);
}

fn agent() -> Agent {
    Agent::new(AgentId::new(1), Name::new("Shell").unwrap(), None)
}

fn node(id: u64) -> Node {
    Node::new(
        NodeId::new(id),
        NodeTarget::Agent(AgentId::new(1)),
        CanvasPoint::new(0.0, 0.0).unwrap(),
        CanvasSize::new(360.0, 260.0).unwrap(),
    )
}

fn workspace() -> Workspace {
    let mut workspace = Workspace::new(WorkspaceId::new(1), Name::new("Test").unwrap());
    workspace
        .execute(DomainCommand::AddAgentNode {
            agent: agent(),
            node: node(1),
        })
        .unwrap();
    workspace
}

#[test]
fn deleting_last_window_removes_active_agent_and_undo_restores_identity() {
    let mut workspace = workspace();
    let before = workspace.canvas_layout();
    for _ in 0..2 {
        workspace
            .execute(DomainCommand::ReplaceCanvas {
                before: before.clone(),
                after: CanvasLayout::default(),
            })
            .unwrap();
        assert_eq!(workspace.agent_count(), 0);
        assert_eq!(workspace.agents().count(), 0);
        assert!(workspace.agent(AgentId::new(1)).is_none());
        assert_eq!(workspace.recorded_agent(AgentId::new(1)), Some(&agent()));
        assert!(workspace.execute(DomainCommand::AddAgent(agent())).is_err());
        workspace
            .execute(DomainCommand::ReplaceCanvas {
                before: CanvasLayout::default(),
                after: before.clone(),
            })
            .unwrap();
        assert_eq!(workspace.agent_count(), 1);
        assert_eq!(workspace.agent(AgentId::new(1)), Some(&agent()));
    }
}

#[test]
fn another_window_keeps_the_agent_registered() {
    let mut workspace = workspace();
    workspace.execute(DomainCommand::AddNode(node(2))).unwrap();
    workspace
        .execute(DomainCommand::ReplaceCanvas {
            before: workspace.canvas_layout(),
            after: CanvasLayout::new(vec![node(2)], vec![], vec![]),
        })
        .unwrap();
    assert_eq!(workspace.agent_count(), 1);
    workspace
        .execute(DomainCommand::ReplaceCanvas {
            before: workspace.canvas_layout(),
            after: CanvasLayout::default(),
        })
        .unwrap();
    assert_eq!(workspace.agent_count(), 0);
    workspace.execute(DomainCommand::AddNode(node(3))).unwrap();
    assert_eq!(workspace.agent_count(), 1);
}

#[test]
fn historical_state_events_remain_replayable_without_reviving_deleted_agents() {
    let mut workspace = workspace();
    workspace
        .execute(DomainCommand::ReplaceCanvas {
            before: workspace.canvas_layout(),
            after: CanvasLayout::default(),
        })
        .unwrap();
    workspace
        .apply(&DomainEvent::AgentStateChanged {
            agent_id: AgentId::new(1),
            from: AgentState::Starting,
            to: AgentState::Stopped,
        })
        .unwrap();
    assert_eq!(workspace.agent_count(), 0);
    assert_eq!(
        workspace.recorded_agent(AgentId::new(1)).unwrap().state(),
        AgentState::Stopped
    );
    assert!(
        workspace
            .execute(DomainCommand::TransitionAgent {
                agent_id: AgentId::new(1),
                to: AgentState::Running,
            })
            .is_err()
    );
}

#[test]
fn deletion_and_saved_conversation_survive_snapshot_and_journal_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("state.sqlite");
    let mut journal = Journal::open(&database).unwrap();
    let mut workspace = Workspace::new(WorkspaceId::new(1), Name::new("Test").unwrap());
    for command in [
        DomainCommand::AddAgentNode {
            agent: agent(),
            node: node(1),
        },
        DomainCommand::AddChatThread(ChatThread::new(
            ChatThreadId::new(1),
            AgentId::new(1),
            Name::new("History").unwrap(),
        )),
        DomainCommand::UpdateChatDraft {
            thread_id: ChatThreadId::new(1),
            draft: ChatDraft::new("Keep this conversation", vec![], vec![]).unwrap(),
        },
        DomainCommand::SubmitChatDraft {
            thread_id: ChatThreadId::new(1),
            message_id: ChatMessageId::new(1),
            sent_at: Timestamp::from_unix_millis(1),
        },
    ] {
        journal
            .execute(&mut workspace, command, Timestamp::from_unix_millis(1))
            .unwrap();
    }
    let before = workspace.canvas_layout();
    journal
        .execute(
            &mut workspace,
            DomainCommand::ReplaceCanvas {
                before,
                after: CanvasLayout::default(),
            },
            Timestamp::from_unix_millis(2),
        )
        .unwrap();
    assert_eq!(journal.recover(workspace.id()).unwrap().unwrap(), workspace);
    drop(journal);
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute("DELETE FROM workspace_snapshots WHERE event_sequence > (SELECT MIN(event_sequence) FROM workspace_snapshots)", []).unwrap();
    drop(connection);
    let journal = Journal::open(&database).unwrap();
    let restored = journal.recover(workspace.id()).unwrap().unwrap();
    assert_eq!(restored, workspace);
    assert_eq!(restored.agent_count(), 0);
    assert_eq!(
        restored
            .chat_thread(ChatThreadId::new(1))
            .unwrap()
            .messages()[0]
            .content()
            .as_str(),
        "Keep this conversation"
    );
}

#[test]
fn loading_legacy_empty_board_cleans_leftovers_but_keeps_standalone_agents() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("state.sqlite");
    let mut manager = WorkspaceManager::open(&database).unwrap();
    let id = manager
        .create_workspace(temp.path(), Timestamp::from_unix_millis(1))
        .unwrap();
    manager
        .execute(
            id,
            DomainCommand::AddAgentNode {
                agent: agent(),
                node: node(1),
            },
            Timestamp::from_unix_millis(2),
        )
        .unwrap();
    manager
        .execute(
            id,
            DomainCommand::AddAgent(Agent::new(
                AgentId::new(2),
                Name::new("Standalone").unwrap(),
                None,
            )),
            Timestamp::from_unix_millis(3),
        )
        .unwrap();
    let before = manager.workspace(id).unwrap().canvas_layout();
    manager
        .execute(
            id,
            DomainCommand::ReplaceCanvas {
                before,
                after: CanvasLayout::default(),
            },
            Timestamp::from_unix_millis(4),
        )
        .unwrap();
    drop(manager);

    // Reproduce the old snapshot format: all agent records remained active.
    let connection = rusqlite::Connection::open(&database).unwrap();
    let (sequence, payload): (u64, Vec<u8>) = connection.query_row(
        "SELECT event_sequence, payload FROM workspace_snapshots ORDER BY event_sequence DESC LIMIT 1",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    let mut stored: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    stored.as_object_mut().unwrap().remove("archived_agents");
    let payload = serde_json::to_vec(&stored).unwrap();
    let version = 11_u32.to_le_bytes();
    let sequence_bytes = sequence.to_le_bytes();
    let workspace_key = id.get().to_string();
    let mut hasher = blake3::Hasher::new();
    for part in [
        &version[..],
        workspace_key.as_bytes(),
        &sequence_bytes,
        &payload,
    ] {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    connection.execute(
        "UPDATE workspace_snapshots SET format_version = 11, payload = ?1, checksum = ?2 WHERE event_sequence = ?3",
        rusqlite::params![payload, hasher.finalize().as_bytes().as_slice(), sequence],
    ).unwrap();
    drop(connection);
    // Prove that this is a valid old snapshot, not a corrupt one skipped on load.
    assert_eq!(
        Journal::open(&database)
            .unwrap()
            .recover(id)
            .unwrap()
            .unwrap()
            .agent_count(),
        2
    );
    for _ in 0..2 {
        let manager = WorkspaceManager::open(&database).unwrap();
        let workspace = manager.workspace(id).unwrap();
        assert_eq!(workspace.agent_count(), 1);
        assert!(workspace.agent(AgentId::new(1)).is_none());
        assert!(workspace.agent(AgentId::new(2)).is_some());
        assert_eq!(workspace.recorded_agents().count(), 2);
    }
}

#[test]
fn portable_import_does_not_reuse_a_deleted_agents_identity() {
    use openpodium::persistence::{
        decode_workspace_archive, export_workspace_archive, import_workspace_archive,
    };
    let source = workspace();
    let archive = decode_workspace_archive(&export_workspace_archive(&source).unwrap()).unwrap();
    let mut destination = workspace();
    destination
        .execute(DomainCommand::ReplaceCanvas {
            before: destination.canvas_layout(),
            after: CanvasLayout::default(),
        })
        .unwrap();
    let plan = import_workspace_archive(&archive, &destination, &std::collections::BTreeMap::new())
        .unwrap();
    for command in plan.commands {
        destination.execute(command).unwrap();
    }
    assert!(destination.agent(AgentId::new(1)).is_none());
    assert!(destination.agent(AgentId::new(2)).is_some());
    assert_eq!(destination.recorded_agents().count(), 2);
    decode_workspace_archive(&export_workspace_archive(&destination).unwrap()).unwrap();
}
