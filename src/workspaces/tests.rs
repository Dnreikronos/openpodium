use std::fs;

use tempfile::TempDir;

use crate::domain::{
    Agent, AgentId, AgentState, CanvasNodeContent, CanvasPoint, CanvasSize, Content, DomainCommand,
    Name, Node, NodeId, NodeTarget, ProjectPath, Role, RoleId, Timestamp, WorkspaceId,
};
use crate::persistence::PointV1;

use super::{WorkspaceError, WorkspaceManager, WorkspaceSettingsInput};

#[test]
fn invalid_workspace_paths_return_actionable_errors() {
    let temp = TempDir::new().unwrap();
    let mut manager = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
    let missing = temp.path().join("missing");

    let missing_error = manager
        .create_workspace(&missing, timestamp(1))
        .unwrap_err();
    assert!(matches!(
        missing_error,
        WorkspaceError::DirectoryAccess {
            operation: "inspect",
            path,
            ..
        } if path == missing
    ));

    let file = temp.path().join("file.txt");
    fs::write(&file, "not a directory").unwrap();
    let file_error = manager.create_workspace(&file, timestamp(2)).unwrap_err();
    assert!(matches!(
        file_error,
        WorkspaceError::NotDirectory { path } if path == file
    ));
}

#[test]
fn workspace_metadata_and_active_selection_survive_restart() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("state.sqlite");
    let first_directory = temp.path().join("first");
    let second_directory = temp.path().join("second");
    fs::create_dir(&first_directory).unwrap();
    fs::create_dir(&second_directory).unwrap();

    let (first_id, second_id) = {
        let mut manager = WorkspaceManager::open(&database).unwrap();
        let first_id = manager
            .create_workspace(&first_directory, timestamp(1))
            .unwrap();
        let second_id = manager
            .create_workspace(&second_directory, timestamp(2))
            .unwrap();
        manager
            .update_settings(
                first_id,
                WorkspaceSettingsInput {
                    name: "OpenPodium".to_owned(),
                    icon: Some("🏛️".to_owned()),
                    working_directory: first_directory.clone(),
                    instructions: Some("Run targeted tests".to_owned()),
                },
                timestamp(3),
            )
            .unwrap();
        manager.switch(first_id, timestamp(4)).unwrap();
        (first_id, second_id)
    };

    let manager = WorkspaceManager::open(&database).unwrap();
    let restored = manager.workspace(first_id).unwrap();

    assert_eq!(manager.active_workspace_id(), Some(first_id));
    assert_eq!(restored.name(), "OpenPodium");
    assert_eq!(restored.settings().icon().unwrap().as_str(), "🏛️");
    assert_eq!(
        restored.settings().instructions().unwrap().as_str(),
        "Run targeted tests"
    );
    assert_eq!(
        restored.settings().working_directory().unwrap().as_str(),
        first_directory.canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(
        manager
            .recent_workspaces()
            .map(|workspace| workspace.id())
            .collect::<Vec<_>>(),
        vec![first_id, second_id]
    );
    assert!(manager.workspace(second_id).is_some());
    assert_eq!(fs::read_dir(&first_directory).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&second_directory).unwrap().count(), 0);
}

#[test]
fn workspace_order_stays_stable_across_selection_and_restart() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("state.sqlite");
    let mut manager = WorkspaceManager::open(&database).unwrap();
    let mut expected = Vec::new();

    for (index, name) in ["zebra", "alpha", "middle"].into_iter().enumerate() {
        let directory = temp.path().join(name);
        fs::create_dir(&directory).unwrap();
        let id = manager
            .create_workspace(&directory, timestamp(index as u64 + 1))
            .unwrap();
        expected.push(id);
        assert_eq!(
            manager
                .ordered_workspaces()
                .map(|workspace| workspace.id())
                .collect::<Vec<_>>(),
            expected
        );
    }

    for (index, id) in [expected[0], expected[2], expected[1], expected[1]]
        .into_iter()
        .enumerate()
    {
        manager.switch(id, timestamp(index as u64 + 4)).unwrap();
        assert_eq!(manager.active_workspace_id(), Some(id));
        assert_eq!(
            manager
                .ordered_workspaces()
                .map(|workspace| workspace.id())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(manager.recent_workspaces().next().unwrap().id(), id);
    }

    drop(manager);
    let manager = WorkspaceManager::open(&database).unwrap();
    assert_eq!(manager.active_workspace_id(), Some(expected[1]));
    assert_eq!(
        manager
            .ordered_workspaces()
            .map(|workspace| workspace.id())
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn switching_workspaces_preserves_running_agents() {
    let temp = TempDir::new().unwrap();
    let first_directory = temp.path().join("first");
    let second_directory = temp.path().join("second");
    fs::create_dir(&first_directory).unwrap();
    fs::create_dir(&second_directory).unwrap();
    let mut manager = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
    let first_id = manager
        .create_workspace(&first_directory, timestamp(1))
        .unwrap();
    let second_id = manager
        .create_workspace(&second_directory, timestamp(2))
        .unwrap();
    let agent_id = AgentId::new(1);

    manager
        .execute(
            first_id,
            DomainCommand::AddAgent(Agent::new(agent_id, Name::new("Builder").unwrap(), None)),
            timestamp(3),
        )
        .unwrap();
    manager
        .execute(
            first_id,
            DomainCommand::TransitionAgent {
                agent_id,
                to: AgentState::Running,
            },
            timestamp(4),
        )
        .unwrap();

    manager.switch(second_id, timestamp(5)).unwrap();
    manager.switch(first_id, timestamp(6)).unwrap();

    assert_eq!(
        manager
            .workspace(first_id)
            .unwrap()
            .agent(agent_id)
            .unwrap()
            .state(),
        AgentState::Running
    );
    assert_eq!(manager.active_workspace_id(), Some(first_id));
    assert!(manager.workspace(WorkspaceId::new(999)).is_none());
}

#[test]
fn template_import_preview_then_batch_import_allocates_fresh_ids() {
    let temp = TempDir::new().unwrap();
    let directory = temp.path().join("project");
    fs::create_dir(&directory).unwrap();
    let mut manager = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
    let workspace_id = manager.create_workspace(&directory, timestamp(1)).unwrap();
    manager
        .execute(
            workspace_id,
            DomainCommand::AddRole(Role::new(
                RoleId::new(50),
                Name::new("Builder").unwrap(),
                Content::new("Build safely").unwrap(),
            )),
            timestamp(2),
        )
        .unwrap();
    manager
        .execute(
            workspace_id,
            DomainCommand::AddAgent(Agent::new(
                AgentId::new(60),
                Name::new("Ada").unwrap(),
                Some(RoleId::new(50)),
            )),
            timestamp(3),
        )
        .unwrap();
    manager
        .execute(
            workspace_id,
            DomainCommand::AddNode(Node::new(
                NodeId::new(70),
                NodeTarget::Agent(AgentId::new(60)),
                CanvasPoint::new(100.0, 200.0).unwrap(),
                CanvasSize::new(240.0, 160.0).unwrap(),
            )),
            timestamp(4),
        )
        .unwrap();

    let payload = manager
        .export_template(workspace_id, &[NodeId::new(70)])
        .unwrap();
    let preview = manager
        .preview_template_import(workspace_id, &payload)
        .unwrap();
    assert_eq!(preview.counts.roles, 1);
    assert_eq!(preview.counts.agents, 1);

    manager
        .import_template(
            workspace_id,
            &payload,
            None,
            PointV1 { x: 500.0, y: 600.0 },
            &std::collections::BTreeMap::new(),
            &std::collections::BTreeMap::new(),
            timestamp(5),
        )
        .unwrap();

    let workspace = manager.workspace(workspace_id).unwrap();
    assert_eq!(workspace.roles().count(), 2);
    assert_eq!(workspace.agents().count(), 2);
    assert_eq!(workspace.all_canvas_layout().nodes().len(), 2);
    assert!(workspace.agent(AgentId::new(61)).is_some());
    assert!(workspace.node(NodeId::new(71)).is_some());
    assert_eq!(
        workspace.node(NodeId::new(71)).unwrap().position().x(),
        500.0
    );
}

#[test]
fn template_import_applies_relative_path_mapping() {
    let temp = TempDir::new().unwrap();
    let directory = temp.path().join("project");
    fs::create_dir(&directory).unwrap();
    let mut manager = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
    let workspace_id = manager.create_workspace(&directory, timestamp(1)).unwrap();
    manager
        .execute(
            workspace_id,
            DomainCommand::AddNode(Node::with_content(
                NodeId::new(1),
                CanvasNodeContent::Note {
                    path: ProjectPath::new("notes/source.md").unwrap(),
                    title: Name::new("Source note").unwrap(),
                },
                CanvasPoint::new(0.0, 0.0).unwrap(),
                CanvasSize::new(240.0, 160.0).unwrap(),
            )),
            timestamp(2),
        )
        .unwrap();
    let payload = manager
        .export_template(workspace_id, &[NodeId::new(1)])
        .unwrap();
    let mut path_mappings = std::collections::BTreeMap::new();
    path_mappings.insert(
        "notes/source.md".to_owned(),
        "notes/destination.md".to_owned(),
    );

    manager
        .import_template(
            workspace_id,
            &payload,
            None,
            PointV1 { x: 400.0, y: 500.0 },
            &std::collections::BTreeMap::new(),
            &path_mappings,
            timestamp(3),
        )
        .unwrap();

    let node = manager
        .workspace(workspace_id)
        .unwrap()
        .node(NodeId::new(2))
        .unwrap();
    match node.content() {
        CanvasNodeContent::Note { path, .. } => assert_eq!(path.as_str(), "notes/destination.md"),
        content => panic!("expected a note, found {content:?}"),
    }
}

#[cfg(unix)]
#[test]
fn template_import_rejects_a_symlinked_destination_path_before_journaling() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let source_directory = temp.path().join("source");
    let destination_directory = temp.path().join("destination");
    let outside_directory = temp.path().join("outside");
    fs::create_dir(&source_directory).unwrap();
    fs::create_dir(&destination_directory).unwrap();
    fs::create_dir(&outside_directory).unwrap();

    let mut manager = WorkspaceManager::open(temp.path().join("state.sqlite")).unwrap();
    let source_id = manager
        .create_workspace(&source_directory, timestamp(1))
        .unwrap();
    let destination_id = manager
        .create_workspace(&destination_directory, timestamp(2))
        .unwrap();
    manager
        .execute(
            source_id,
            DomainCommand::AddNode(Node::with_content(
                NodeId::new(1),
                CanvasNodeContent::Note {
                    path: ProjectPath::new(".openpodium/notes/brief.md").unwrap(),
                    title: Name::new("Brief").unwrap(),
                },
                CanvasPoint::new(0.0, 0.0).unwrap(),
                CanvasSize::new(240.0, 160.0).unwrap(),
            )),
            timestamp(3),
        )
        .unwrap();
    let payload = manager
        .export_template(source_id, &[NodeId::new(1)])
        .unwrap();
    symlink(
        &outside_directory,
        destination_directory.join(".openpodium"),
    )
    .unwrap();

    let error = manager
        .import_template(
            destination_id,
            &payload,
            None,
            PointV1 { x: 0.0, y: 0.0 },
            &std::collections::BTreeMap::new(),
            &std::collections::BTreeMap::new(),
            timestamp(4),
        )
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::Portable(_)));
    assert!(
        manager
            .workspace(destination_id)
            .unwrap()
            .all_canvas_layout()
            .nodes()
            .is_empty()
    );
}

fn timestamp(value: u64) -> Timestamp {
    Timestamp::from_unix_millis(value)
}
