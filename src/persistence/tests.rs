use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use tempfile::TempDir;

use crate::domain::{
    Agent, AgentId, AgentState, CanvasPoint, CanvasSize, Content, DomainCommand, Handoff,
    HandoffId, HandoffPayload, Name, Node, NodeId, NodeTarget, Role, RoleId, Task, TaskId,
    TaskState, Timestamp, Workspace, WorkspaceId,
};

use super::{Journal, PersistenceError};

#[test]
fn new_database_enables_wal_foreign_keys_and_schema_version() {
    let temp = TempDir::new().unwrap();
    let journal = Journal::open(database_path(&temp)).unwrap();

    let schema_version: i64 = journal
        .connection()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    let journal_mode: String = journal
        .connection()
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    let foreign_keys: i64 = journal
        .connection()
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .unwrap();

    assert_eq!(schema_version, 1);
    assert_eq!(journal_mode, "wal");
    assert_eq!(foreign_keys, 1);
}

#[test]
fn failed_snapshot_write_rolls_back_event_and_memory() {
    let temp = TempDir::new().unwrap();
    let mut journal = Journal::open(database_path(&temp)).unwrap();
    let mut workspace = test_workspace();
    let before = workspace.clone();
    journal
        .connection()
        .execute_batch(
            "CREATE TRIGGER reject_snapshot
             BEFORE INSERT ON workspace_snapshots
             BEGIN
                 SELECT RAISE(ABORT, 'simulated interrupted write');
             END;",
        )
        .unwrap();

    let error = journal
        .execute(
            &mut workspace,
            DomainCommand::AddRole(test_role()),
            timestamp(1),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        PersistenceError::Database {
            operation: "insert workspace snapshot",
            ..
        }
    ));
    assert_eq!(workspace, before);
    assert_eq!(table_count(&journal, "journal_events"), 0);
    assert_eq!(table_count(&journal, "workspace_snapshots"), 0);

    journal
        .connection()
        .execute_batch("DROP TRIGGER reject_snapshot")
        .unwrap();
    journal
        .execute(
            &mut workspace,
            DomainCommand::AddRole(test_role()),
            timestamp(2),
        )
        .unwrap();
    assert_eq!(table_count(&journal, "journal_events"), 1);
    assert_eq!(table_count(&journal, "workspace_snapshots"), 1);
}

#[test]
fn restart_restores_the_same_domain_state() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let expected = {
        let mut journal = Journal::open(&path).unwrap();
        let mut workspace = test_workspace();

        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::AddRole(test_role()),
            1,
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::AddAgent(Agent::new(
                AgentId::new(1),
                name("Ada"),
                Some(RoleId::new(1)),
            )),
            2,
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::AddAgent(Agent::new(
                AgentId::new(2),
                name("Linus"),
                Some(RoleId::new(1)),
            )),
            3,
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::TransitionAgent {
                agent_id: AgentId::new(1),
                to: AgentState::Running,
            },
            4,
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::AddTask(Task::new(
                TaskId::new(1),
                name("Build persistence"),
                content("Persist the domain journal"),
                Some(AgentId::new(1)),
                None,
            )),
            5,
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::TransitionTask {
                task_id: TaskId::new(1),
                to: TaskState::Delivered,
            },
            6,
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::AddHandoff(Handoff::new(
                HandoffId::new(1),
                AgentId::new(2),
                AgentId::new(1),
                HandoffPayload::Task(TaskId::new(1)),
            )),
            7,
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::AddNode(Node::new(
                NodeId::new(1),
                NodeTarget::Task(TaskId::new(1)),
                CanvasPoint::new(10.0, 20.0).unwrap(),
                CanvasSize::new(400.0, 240.0).unwrap(),
            )),
            8,
        );

        workspace
    };

    let journal = Journal::open(&path).unwrap();
    let recovered = journal.recover(WorkspaceId::new(7)).unwrap().unwrap();

    assert_eq!(recovered, expected);
}

#[test]
fn corrupt_newest_snapshot_is_skipped_and_its_event_is_replayed() {
    let temp = TempDir::new().unwrap();
    let mut journal = Journal::open(database_path(&temp)).unwrap();
    let mut workspace = test_workspace();
    persist(
        &mut journal,
        &mut workspace,
        DomainCommand::AddRole(test_role()),
        1,
    );
    persist(
        &mut journal,
        &mut workspace,
        DomainCommand::AddAgent(Agent::new(
            AgentId::new(1),
            name("Ada"),
            Some(RoleId::new(1)),
        )),
        2,
    );
    let expected = workspace.clone();
    journal
        .connection()
        .execute(
            "UPDATE workspace_snapshots SET checksum = zeroblob(32)
             WHERE event_sequence = 2",
            [],
        )
        .unwrap();

    let recovered = journal.recover(workspace.id()).unwrap().unwrap();

    assert_eq!(recovered, expected);
}

#[test]
fn corrupt_event_needed_for_replay_fails_recovery() {
    let temp = TempDir::new().unwrap();
    let mut journal = Journal::open(database_path(&temp)).unwrap();
    let mut workspace = test_workspace();
    persist(
        &mut journal,
        &mut workspace,
        DomainCommand::AddRole(test_role()),
        1,
    );
    persist(
        &mut journal,
        &mut workspace,
        DomainCommand::AddAgent(Agent::new(
            AgentId::new(1),
            name("Ada"),
            Some(RoleId::new(1)),
        )),
        2,
    );
    journal
        .connection()
        .execute_batch(
            "UPDATE workspace_snapshots SET checksum = zeroblob(32)
                 WHERE event_sequence = 2;
             UPDATE journal_events SET payload = X'00'
                 WHERE sequence = 2;",
        )
        .unwrap();

    let error = journal.recover(workspace.id()).unwrap_err();

    assert!(matches!(
        error,
        PersistenceError::ChecksumMismatch {
            record_type: "domain event",
            sequence: 2,
        }
    ));
}

#[test]
fn unknown_event_format_fails_recovery_without_partial_state() {
    let temp = TempDir::new().unwrap();
    let mut journal = Journal::open(database_path(&temp)).unwrap();
    let mut workspace = test_workspace();
    persist(
        &mut journal,
        &mut workspace,
        DomainCommand::AddRole(test_role()),
        1,
    );
    persist(
        &mut journal,
        &mut workspace,
        DomainCommand::AddAgent(Agent::new(
            AgentId::new(1),
            name("Ada"),
            Some(RoleId::new(1)),
        )),
        2,
    );
    journal
        .connection()
        .execute_batch(
            "UPDATE workspace_snapshots SET checksum = zeroblob(32)
                 WHERE event_sequence = 2;
             UPDATE journal_events SET format_version = 99
                 WHERE sequence = 2;",
        )
        .unwrap();

    let error = journal.recover(workspace.id()).unwrap_err();

    assert!(matches!(
        error,
        PersistenceError::UnsupportedRecordVersion {
            record_type: "domain event",
            sequence: 2,
            found: 99,
            supported: 1,
        }
    ));
}

#[test]
fn empty_journal_has_no_workspace_to_recover() {
    let temp = TempDir::new().unwrap();
    let journal = Journal::open(database_path(&temp)).unwrap();

    assert_eq!(journal.recover(WorkspaceId::new(7)).unwrap(), None);
}

#[test]
fn database_with_only_invalid_snapshot_reports_recovery_error() {
    let temp = TempDir::new().unwrap();
    let mut journal = Journal::open(database_path(&temp)).unwrap();
    let mut workspace = test_workspace();
    persist(
        &mut journal,
        &mut workspace,
        DomainCommand::AddRole(test_role()),
        1,
    );
    journal
        .connection()
        .execute("UPDATE workspace_snapshots SET payload = X'00'", [])
        .unwrap();

    let error = journal.recover(workspace.id()).unwrap_err();

    assert!(matches!(
        error,
        PersistenceError::NoValidSnapshot {
            workspace_id,
            snapshots_checked: 1,
        } if workspace_id == workspace.id()
    ));
}

#[test]
fn unknown_schema_version_does_not_modify_the_database() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    {
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE user_data (value TEXT NOT NULL);
                 INSERT INTO user_data VALUES ('keep me');
                 PRAGMA user_version = 99;",
            )
            .unwrap();
    }
    let before = fs::read(&path).unwrap();

    let error = Journal::open(&path).err().unwrap();

    assert!(matches!(
        error,
        PersistenceError::UnsupportedSchemaVersion {
            found: 99,
            supported: 1,
        }
    ));
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn migrating_non_empty_database_creates_a_consistent_backup() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    {
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE legacy_data (value TEXT NOT NULL);
                 INSERT INTO legacy_data VALUES ('preserved');",
            )
            .unwrap();
    }

    let journal = Journal::open(&path).unwrap();
    let backup_path = migration_backup_path(&path);
    let backup = Connection::open(&backup_path).unwrap();
    let backup_version: i64 = backup
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    let backup_value: String = backup
        .query_row("SELECT value FROM legacy_data", [], |row| row.get(0))
        .unwrap();
    let current_value: String = journal
        .connection()
        .query_row("SELECT value FROM legacy_data", [], |row| row.get(0))
        .unwrap();

    assert_eq!(backup_version, 0);
    assert_eq!(backup_value, "preserved");
    assert_eq!(current_value, "preserved");
}

#[test]
fn migration_failure_reports_versions_and_preserves_a_backup() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    {
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE journal_events (legacy_value TEXT NOT NULL);
                 INSERT INTO journal_events VALUES ('preserved');",
            )
            .unwrap();
    }

    let error = Journal::open(&path).err().unwrap();
    let backup_path = match error {
        PersistenceError::Migration {
            from: 0,
            to: 1,
            backup_path: Some(path),
            ..
        } => path,
        other => panic!("unexpected error: {other}"),
    };

    assert!(backup_path.exists());
    let original = Connection::open(&path).unwrap();
    let original_version: i64 = original
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    let original_value: String = original
        .query_row("SELECT legacy_value FROM journal_events", [], |row| {
            row.get(0)
        })
        .unwrap();
    let backup = Connection::open(backup_path).unwrap();
    let backup_value: String = backup
        .query_row("SELECT legacy_value FROM journal_events", [], |row| {
            row.get(0)
        })
        .unwrap();

    assert_eq!(original_version, 0);
    assert_eq!(original_value, "preserved");
    assert_eq!(backup_value, "preserved");
}

fn persist(journal: &mut Journal, workspace: &mut Workspace, command: DomainCommand, at: u64) {
    journal.execute(workspace, command, timestamp(at)).unwrap();
}

fn table_count(journal: &Journal, table: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    journal
        .connection()
        .query_row(&sql, [], |row| row.get(0))
        .unwrap()
}

fn test_workspace() -> Workspace {
    Workspace::new(WorkspaceId::new(7), name("Persistence test"))
}

fn test_role() -> Role {
    Role::new(
        RoleId::new(1),
        name("Builder"),
        content("Implement the requested change"),
    )
}

fn name(value: &str) -> Name {
    Name::new(value).unwrap()
}

fn content(value: &str) -> Content {
    Content::new(value).unwrap()
}

fn timestamp(value: u64) -> Timestamp {
    Timestamp::from_unix_millis(value)
}

fn database_path(temp: &TempDir) -> PathBuf {
    temp.path().join("journal.sqlite")
}

fn migration_backup_path(database: &Path) -> PathBuf {
    database.with_file_name("journal.sqlite.backup-v0-before-v1")
}
