use std::fs;

use tempfile::TempDir;

use crate::domain::{Agent, AgentId, AgentState, DomainCommand, Name, Timestamp, WorkspaceId};

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

fn timestamp(value: u64) -> Timestamp {
    Timestamp::from_unix_millis(value)
}
