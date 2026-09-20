use super::*;
use crate::domain::{Agent, AgentId, CanvasPoint, CanvasSize, Node, NodeId};

fn setup() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    WorkspaceManager,
    WorkspaceId,
) {
    let project = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-b", "main"],
        vec!["config", "user.name", "Test"],
        vec!["config", "user.email", "test@example.com"],
        vec![
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
    ] {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(project.path())
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let storage = tempfile::tempdir().unwrap();
    let mut manager = WorkspaceManager::open(storage.path().join("state.db")).unwrap();
    let id = manager
        .create_workspace(project.path(), Timestamp::from_unix_millis(1))
        .unwrap();
    (project, storage, manager, id)
}

fn add_node(manager: &mut WorkspaceManager, workspace: WorkspaceId, id: u64) {
    manager
        .execute(
            workspace,
            DomainCommand::AddAgentNode {
                agent: Agent::new(
                    AgentId::new(id),
                    Name::new(format!("Agent {id}")).unwrap(),
                    None,
                ),
                node: Node::new(
                    NodeId::new(id),
                    NodeTarget::Agent(AgentId::new(id)),
                    CanvasPoint::new(10.0, 20.0).unwrap(),
                    CanvasSize::new(300.0, 200.0).unwrap(),
                ),
            },
            Timestamp::from_unix_millis(2),
        )
        .unwrap();
}

#[test]
fn worktree_floor_restores_owner_selection_and_independent_canvas() {
    let (_project, storage, mut manager, id) = setup();
    let at = Timestamp::from_unix_millis(3);
    add_node(&mut manager, id, 1);
    let floor = manager
        .create_floor(
            id,
            "task",
            "feature/task",
            Some(FloorOwner::Agent(AgentId::new(1))),
            at,
        )
        .unwrap();
    assert!(
        manager
            .workspace(id)
            .unwrap()
            .canvas_layout()
            .nodes()
            .is_empty()
    );
    add_node(&mut manager, id, 2);
    let workspace = manager.workspace(id).unwrap();
    let directory = workspace.node_directory(NodeId::new(2)).unwrap().clone();
    assert_ne!(
        workspace.node_directory(NodeId::new(1)).unwrap(),
        &directory
    );
    let before = workspace.canvas_layout();
    let after = crate::domain::CanvasLayout::new(vec![], vec![], vec![]);
    manager
        .execute(
            id,
            DomainCommand::ReplaceCanvas {
                before: before.clone(),
                after: after.clone(),
            },
            at,
        )
        .unwrap();
    assert!(
        manager
            .workspace(id)
            .unwrap()
            .node(NodeId::new(1))
            .is_some()
    );
    manager
        .execute(
            id,
            DomainCommand::ReplaceCanvas {
                before: after,
                after: before,
            },
            at,
        )
        .unwrap();
    drop(manager);
    let mut restored = WorkspaceManager::open(storage.path().join("state.db")).unwrap();
    let workspace = restored.workspace(id).unwrap();
    assert_eq!(workspace.floors().active, Some(floor));
    assert_eq!(workspace.canvas_layout().nodes()[0].id(), NodeId::new(2));
    assert_eq!(workspace.node_directory(NodeId::new(2)), Some(&directory));
    assert_eq!(
        workspace.floors().entries[&floor].owner,
        Some(FloorOwner::Agent(AgentId::new(1)))
    );
    restored.switch_floor(id, None, at).unwrap();
    assert_eq!(
        restored.workspace(id).unwrap().canvas_layout().nodes()[0].id(),
        NodeId::new(1)
    );
    assert_eq!(
        restored
            .workspace(id)
            .unwrap()
            .node_directory(NodeId::new(2)),
        Some(&directory)
    );
    assert!(
        restored
            .remove_floor(id, floor, true, at)
            .unwrap_err()
            .contains("Stop all agents")
    );
}

#[test]
fn discovered_worktrees_remain_user_owned_after_restart() {
    let (project, storage, mut manager, id) = setup();
    let parent = tempfile::tempdir().unwrap();
    let repo = Repository::discover(project.path()).unwrap();
    let checkout = repo.create("external", "external", parent.path()).unwrap();
    let at = Timestamp::from_unix_millis(2);
    manager.refresh_floors(id, at).unwrap();
    let floor = *manager
        .workspace(id)
        .unwrap()
        .floors()
        .entries
        .keys()
        .next()
        .unwrap();
    assert!(!manager.workspace(id).unwrap().floors().entries[&floor].managed);
    drop(manager);
    let mut restored = WorkspaceManager::open(storage.path().join("state.db")).unwrap();
    assert!(restored.remove_floor(id, floor, true, at).is_err());
    assert!(checkout.path.exists());
}

#[test]
fn missing_checkouts_block_launch_and_cleanup_preserves_history() {
    let (_project, _storage, mut manager, id) = setup();
    let at = Timestamp::from_unix_millis(3);
    let floor = manager.create_floor(id, "task", "task", None, at).unwrap();
    let path = manager.workspace(id).unwrap().floors().entries[&floor]
        .directory
        .as_str()
        .to_owned();
    std::fs::write(Path::new(&path).join("work"), "untracked").unwrap();
    manager.refresh_floors(id, at).unwrap();
    assert!(manager.workspace(id).unwrap().floors().entries[&floor].dirty);
    assert!(manager.remove_floor(id, floor, false, at).is_err());
    manager.remove_floor(id, floor, true, at).unwrap();
    assert_eq!(
        manager.workspace(id).unwrap().floors().entries[&floor].lifecycle,
        FloorLifecycle::Removed
    );
    assert_eq!(manager.workspace(id).unwrap().floors().active, None);
    assert!(!Path::new(&path).exists());
}

#[test]
fn replacement_checkout_never_inherits_managed_ownership() {
    let (project, _storage, mut manager, id) = setup();
    let at = Timestamp::from_unix_millis(3);
    let floor = manager.create_floor(id, "task", "task", None, at).unwrap();
    let path = std::path::PathBuf::from(
        manager.workspace(id).unwrap().floors().entries[&floor]
            .directory
            .as_str(),
    );
    for args in [
        vec!["worktree", "remove", path.to_str().unwrap()],
        vec!["worktree", "add", path.to_str().unwrap(), "task"],
    ] {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(project.path())
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    assert!(
        manager
            .remove_floor(id, floor, true, at)
            .unwrap_err()
            .contains("ownership changed")
    );
    manager.refresh_floors(id, at).unwrap();
    assert!(!manager.workspace(id).unwrap().floors().entries[&floor].managed);
    assert!(manager.remove_floor(id, floor, true, at).is_err());
    assert!(path.exists());
}

#[test]
fn background_git_result_keeps_canvas_edits_made_while_it_runs() {
    let (_project, _storage, mut manager, id) = setup();
    let at = Timestamp::from_unix_millis(3);
    let floor = manager.create_floor(id, "task", "task", None, at).unwrap();
    let job = manager
        .prepare_floor_operation(id, FloorOperation::Refresh)
        .unwrap();
    add_node(&mut manager, id, 1);
    manager
        .finish_floor_operation(job.run().unwrap(), at)
        .unwrap();
    let workspace = manager.workspace(id).unwrap();
    assert_eq!(workspace.floors().node_floors[&NodeId::new(1)], floor);
    assert_eq!(workspace.canvas_layout().nodes().len(), 1);
}

#[test]
fn missing_floor_preserves_canvas_but_has_no_launch_directory() {
    let (_project, _storage, mut manager, id) = setup();
    let at = Timestamp::from_unix_millis(3);
    let floor = manager.create_floor(id, "task", "task", None, at).unwrap();
    add_node(&mut manager, id, 1);
    let path = manager.workspace(id).unwrap().floors().entries[&floor]
        .directory
        .as_str()
        .to_owned();
    std::fs::remove_dir_all(&path).unwrap();
    manager.refresh_floors(id, at).unwrap();
    let workspace = manager.workspace(id).unwrap();
    assert_eq!(
        workspace.floors().entries[&floor].lifecycle,
        FloorLifecycle::Missing
    );
    assert_eq!(workspace.canvas_layout().nodes().len(), 1);
    assert!(workspace.node_directory(NodeId::new(1)).is_none());
}

#[cfg(windows)]
#[test]
fn floor_identity_ignores_windows_separator_spelling() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = dunce::canonicalize(directory.path()).unwrap();
    let alternate = std::path::PathBuf::from(canonical.to_string_lossy().replace('\\', "/"));
    assert!(same_path(&canonical, &alternate));
}

#[test]
fn task_floor_recovers_by_replaying_events_after_an_older_snapshot() {
    let (_project, storage, mut manager, id) = setup();
    let at = Timestamp::from_unix_millis(3);
    let task = crate::domain::TaskId::new(1);
    manager
        .execute(
            id,
            DomainCommand::AddTask(crate::domain::Task::new(
                task,
                Name::new("Task").unwrap(),
                crate::domain::Content::new("Work").unwrap(),
                None,
                None,
            )),
            at,
        )
        .unwrap();
    let floor = manager
        .create_floor(id, "task", "task", Some(FloorOwner::Task(task)), at)
        .unwrap();
    add_node(&mut manager, id, 1);
    let expected = manager.workspace(id).unwrap().clone();
    drop(manager);
    let connection = rusqlite::Connection::open(storage.path().join("state.db")).unwrap();
    connection.execute("DELETE FROM workspace_snapshots WHERE event_sequence > (SELECT MIN(event_sequence) FROM workspace_snapshots)", []).unwrap();
    drop(connection);
    let restored = WorkspaceManager::open(storage.path().join("state.db")).unwrap();
    assert_eq!(restored.workspace(id).unwrap(), &expected);
    assert_eq!(
        restored.workspace(id).unwrap().floors().entries[&floor].owner,
        Some(FloorOwner::Task(task))
    );
}
