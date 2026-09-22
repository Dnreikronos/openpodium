use super::*;

#[test]
fn transcript_flush_retries_unchanged_output_after_a_storage_failure() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("state.sqlite");
    let mut manager = WorkspaceManager::open(&database).unwrap();
    let workspace_id = manager.create_workspace(temp.path(), now()).unwrap();
    let key = TerminalKey {
        workspace_id,
        node_id: NodeId::new(1),
    };
    let mut session = Session::starting(terminal::GridSize::for_node(360.0, 260.0), 1);
    session.handle_event(
        ProcessEvent::Output(b"Retain this output".to_vec()),
        Default::default(),
    );
    let mut state = tests::test_state(manager, BTreeMap::from([(key, session)]));
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_transcript BEFORE INSERT ON terminal_transcripts BEGIN SELECT RAISE(FAIL, 'test storage failure'); END;").unwrap();
    state.flush_terminal_transcripts();
    assert!(
        state
            .terminals
            .get_mut(&key)
            .unwrap()
            .take_transcript()
            .is_some()
    );
    assert!(
        state
            .workspaces
            .as_ref()
            .unwrap()
            .terminal_transcripts(workspace_id)
            .unwrap()
            .is_empty()
    );
    connection
        .execute_batch("DROP TRIGGER reject_transcript;")
        .unwrap();
    state.flush_terminal_transcripts();
    assert!(
        state
            .terminals
            .get_mut(&key)
            .unwrap()
            .take_transcript()
            .is_none()
    );
    let stored = state
        .workspaces
        .as_ref()
        .unwrap()
        .terminal_transcripts(workspace_id)
        .unwrap();
    assert_eq!(stored.len(), 1);
    let mut expected = state.terminals[&key].view();
    expected.status = terminal::Status::Offline;
    assert_eq!(
        Session::restored(expected.size, 2, stored[0].1.clone()).view(),
        expected
    );
}

#[test]
fn transcript_restore_keeps_windows_on_inactive_floors() {
    use openpodium::domain::{Floor, FloorLifecycle};
    let temp = tempfile::tempdir().unwrap();
    let mut manager = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
    let workspace_id = manager.create_workspace(temp.path(), now()).unwrap();
    let node = |id| {
        Node::new(
            NodeId::new(id),
            NodeTarget::Agent(AgentId::new(1)),
            CanvasPoint::new(0.0, 0.0).unwrap(),
            CanvasSize::new(360.0, 260.0).unwrap(),
        )
    };
    manager
        .execute(
            workspace_id,
            DomainCommand::AddAgentNode {
                agent: Agent::new(AgentId::new(1), Name::new("Shell").unwrap(), None),
                node: node(1),
            },
            now(),
        )
        .unwrap();
    manager
        .execute(workspace_id, DomainCommand::AddNode(node(2)), now())
        .unwrap();
    let before = manager.workspace(workspace_id).unwrap().floors().clone();
    let mut after = before.clone();
    let directory =
        openpodium::domain::WorkspaceDirectory::new(temp.path().to_str().unwrap()).unwrap();
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
    manager
        .execute(
            workspace_id,
            DomainCommand::ReplaceFloors { before, after },
            now(),
        )
        .unwrap();
    for id in [1, 2, 3] {
        manager
            .store_terminal_transcript(workspace_id, id, now(), b"Saved output")
            .unwrap();
    }
    let mut state = tests::test_state(manager, BTreeMap::new());
    state.restore_terminal_transcripts();
    for id in [1, 2] {
        assert!(state.terminals.contains_key(&TerminalKey {
            workspace_id,
            node_id: NodeId::new(id)
        }));
    }
    let transcripts = state
        .workspaces
        .as_ref()
        .unwrap()
        .terminal_transcripts(workspace_id)
        .unwrap();
    assert_eq!(transcripts.len(), 2);
    assert!(transcripts.iter().any(|(id, _)| *id == 2));
    assert!(!transcripts.iter().any(|(id, _)| *id == 3));
}
