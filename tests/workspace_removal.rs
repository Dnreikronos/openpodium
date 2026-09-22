use openpodium::domain::{DomainCommand, Name, Timestamp, WorkspaceSettings};
use openpodium::persistence::Journal;
use openpodium::workspaces::WorkspaceManager;

#[test]
fn removing_active_inactive_and_last_workspaces_persists_and_reserves_ids() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    let mut manager = WorkspaceManager::open(&path).unwrap();
    let time = Timestamp::from_unix_millis(1);
    let first = manager.create_workspace(temp.path(), time).unwrap();
    let second = manager.create_workspace(temp.path(), time).unwrap();
    let third = manager.create_workspace(temp.path(), time).unwrap();
    manager.remove_workspace(first).unwrap();
    assert_eq!(manager.active_workspace_id(), Some(third));
    manager.remove_workspace(third).unwrap();
    assert_eq!(manager.active_workspace_id(), Some(second));
    manager.remove_workspace(second).unwrap();
    assert!(manager.active_workspace().is_none());
    drop(manager);
    let mut manager = WorkspaceManager::open(&path).unwrap();
    assert_eq!(manager.ordered_workspaces().count(), 0);
    assert!(manager.active_workspace().is_none());
    let other = temp.path().join("other");
    std::fs::create_dir(&other).unwrap();
    assert!(manager.create_workspace(other, time).unwrap().get() > third.get());
}

#[test]
fn opening_a_removed_folder_restores_its_identity_settings_and_history() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let file = project.join("keep.md");
    std::fs::write(&file, "Keep my files").unwrap();
    let path = temp.path().join("state.sqlite");
    let mut manager = WorkspaceManager::open(&path).unwrap();
    let time = Timestamp::from_unix_millis(1);
    let id = manager.create_workspace(&project, time).unwrap();
    let directory = manager
        .workspace(id)
        .unwrap()
        .settings()
        .working_directory()
        .cloned();
    manager
        .execute(
            id,
            DomainCommand::UpdateWorkspaceSettings(WorkspaceSettings::new(
                Name::new("Custom name").unwrap(),
                None,
                directory,
                None,
            )),
            time,
        )
        .unwrap();
    let expected = manager.workspace(id).unwrap().clone();
    manager.remove_workspace(id).unwrap();
    drop(manager);
    assert_eq!(std::fs::read_to_string(file).unwrap(), "Keep my files");
    assert_eq!(
        Journal::open(&path).unwrap().recover(id).unwrap(),
        Some(expected.clone())
    );
    let mut manager = WorkspaceManager::open(&path).unwrap();
    assert_eq!(manager.create_workspace(&project, time).unwrap(), id);
    assert_eq!(manager.workspace(id), Some(&expected));
    assert_eq!(manager.active_workspace_id(), Some(id));
}

#[test]
fn failed_removal_rolls_back_registry_and_active_selection() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    let mut manager = WorkspaceManager::open(&path).unwrap();
    let time = Timestamp::from_unix_millis(1);
    let id = manager.create_workspace(temp.path(), time).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_removal BEFORE DELETE ON workspace_registry BEGIN SELECT RAISE(FAIL, 'test failure'); END;").unwrap();
    assert!(manager.remove_workspace(id).is_err());
    assert_eq!(manager.active_workspace_id(), Some(id));
    assert!(manager.workspace(id).is_some());
    let restored = WorkspaceManager::open(&path).unwrap();
    assert_eq!(restored.active_workspace_id(), Some(id));
    assert_eq!(restored.ordered_workspaces().count(), 1);
}
