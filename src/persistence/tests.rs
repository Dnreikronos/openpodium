use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use tempfile::TempDir;

use crate::domain::{
    Agent, AgentId, AgentProgram, AgentState, CanvasLayout, CanvasPoint, CanvasSize,
    ChatAttachment, ChatAttachmentId, ChatAuthor, ChatDraft, ChatMessage, ChatMessageId,
    ChatThread, ChatThreadId, CommandPreset, CommandPresetId, Connection as DomainConnection,
    ConnectionId, ConnectionKind, Content, DeliveryMechanism, DomainCommand, DomainEvent,
    EnvironmentKind, EnvironmentProfile, EnvironmentProfileId, Handoff, HandoffId,
    HandoffMessageId, HandoffPayload, HandoffProgress, HandoffResponse, HandoffResponseStatus,
    Name, Node, NodeGroup, NodeGroupId, NodeId, NodeTarget, Role, RoleColor, RoleIcon, RoleId,
    SshEnvironment, Task, TaskId, TaskState, ThreadColor, Timestamp, Workspace, WorkspaceId,
};

use super::codec::{EVENT_FORMAT_VERSION, decode_event, decode_workspace};
use super::{Journal, PersistenceError, RoleTransferError, export_role, import_role};

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

    assert_eq!(schema_version, 2);
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
fn typed_handoff_history_survives_restart() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let expected = {
        let mut journal = Journal::open(&path).unwrap();
        let mut workspace = test_workspace();
        for (id, agent_name) in [(1, "Lead"), (2, "Builder")] {
            persist(
                &mut journal,
                &mut workspace,
                DomainCommand::AddAgent(Agent::new(AgentId::new(id), name(agent_name), None)),
                id,
            );
        }
        let task = Task::new(
            TaskId::new(1),
            name("Persist handoffs"),
            content("Keep every delivery fact"),
            Some(AgentId::new(2)),
            None,
        );
        let handoff = Handoff::tracked(
            HandoffId::new(1),
            HandoffMessageId::new("task-1").unwrap(),
            AgentId::new(1),
            AgentId::new(2),
            HandoffPayload::Task(task.id()),
            None,
            timestamp(3),
            Some(timestamp(100)),
        )
        .unwrap();
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::AddTaskHandoff { task, handoff },
            3,
        );

        let before = workspace.handoff(HandoffId::new(1)).unwrap().clone();
        let mut started = before.clone();
        started
            .begin_delivery(
                HandoffMessageId::new("task-1").unwrap(),
                DeliveryMechanism::CodexTerminal,
                timestamp(4),
            )
            .unwrap();
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::UpdateHandoff {
                before,
                after: started.clone(),
            },
            4,
        );
        let mut delivered = started.clone();
        delivered.complete_delivery(1, timestamp(5)).unwrap();
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::UpdateHandoff {
                before: started,
                after: delivered.clone(),
            },
            5,
        );
        let mut progressed = delivered.clone();
        progressed
            .report_progress(HandoffProgress::new(
                HandoffMessageId::new("progress-1").unwrap(),
                content("Halfway"),
                timestamp(6),
            ))
            .unwrap();
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::UpdateHandoff {
                before: delivered,
                after: progressed.clone(),
            },
            6,
        );
        let mut responded = progressed.clone();
        responded
            .respond(HandoffResponse::new(
                HandoffMessageId::new("response-1").unwrap(),
                HandoffResponseStatus::Completed,
                content("Done"),
                timestamp(7),
            ))
            .unwrap();
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::UpdateHandoff {
                before: progressed,
                after: responded,
            },
            7,
        );
        workspace
    };

    let recovered = Journal::open(&path)
        .unwrap()
        .recover(WorkspaceId::new(7))
        .unwrap()
        .unwrap();
    assert_eq!(recovered, expected);
    assert_eq!(
        recovered
            .handoff(HandoffId::new(1))
            .unwrap()
            .response()
            .unwrap()
            .body()
            .as_str(),
        "Done"
    );
}

#[test]
fn canvas_graph_and_agent_program_survive_restart() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let expected = {
        let mut journal = Journal::open(&path).unwrap();
        let mut workspace = test_workspace();
        for (id, program) in [(1, AgentProgram::Codex), (2, AgentProgram::OpenCode)] {
            persist(
                &mut journal,
                &mut workspace,
                DomainCommand::AddAgent(Agent::with_program(
                    AgentId::new(id),
                    name(if id == 1 { "Codex" } else { "OpenCode" }),
                    None,
                    program,
                )),
                id,
            );
        }
        let after = CanvasLayout::new(
            vec![
                Node::with_z_index(
                    NodeId::new(1),
                    NodeTarget::Agent(AgentId::new(1)),
                    CanvasPoint::new(-160.0, -120.0).unwrap(),
                    CanvasSize::new(320.0, 240.0).unwrap(),
                    3,
                ),
                Node::with_z_index(
                    NodeId::new(2),
                    NodeTarget::Agent(AgentId::new(2)),
                    CanvasPoint::new(240.0, -120.0).unwrap(),
                    CanvasSize::new(320.0, 240.0).unwrap(),
                    4,
                ),
            ],
            vec![NodeGroup::new(
                NodeGroupId::new(1),
                [NodeId::new(1), NodeId::new(2)],
            )],
            vec![DomainConnection::new(
                ConnectionId::new(1),
                NodeId::new(1),
                NodeId::new(2),
                ConnectionKind::Coordination,
            )],
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::ReplaceCanvas {
                before: CanvasLayout::default(),
                after,
            },
            3,
        );
        workspace
    };

    let recovered = Journal::open(&path)
        .unwrap()
        .recover(WorkspaceId::new(7))
        .unwrap()
        .unwrap();

    assert_eq!(recovered, expected);
    assert_eq!(
        recovered.agent(AgentId::new(1)).unwrap().program(),
        AgentProgram::Codex
    );
}

#[test]
fn environment_profiles_and_agent_references_survive_restart() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let profile_id = EnvironmentProfileId::new(1);
    let expected = {
        let mut journal = Journal::open(&path).unwrap();
        let mut workspace = test_workspace();
        let profile = EnvironmentProfile::new(
            profile_id,
            name("Remote builder"),
            EnvironmentKind::Ssh(
                SshEnvironment::new(
                    "builder.example.com",
                    Some("codex".to_owned()),
                    Some(2222),
                    "/workspace",
                )
                .unwrap(),
            ),
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::AddEnvironmentProfile(profile),
            1,
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::AddAgent(
                Agent::with_program(
                    AgentId::new(1),
                    name("Remote Codex"),
                    None,
                    AgentProgram::Codex,
                )
                .in_environment(profile_id),
            ),
            2,
        );
        workspace
    };

    let journal = Journal::open(&path).unwrap();
    let recovered = journal.recover(WorkspaceId::new(7)).unwrap().unwrap();

    assert_eq!(recovered, expected);
    assert_eq!(
        recovered.agent(AgentId::new(1)).unwrap().environment_id(),
        Some(profile_id)
    );
}

#[test]
fn command_presets_role_appearance_and_assignments_survive_restart() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let role_id = RoleId::new(1);
    let expected = {
        let mut journal = Journal::open(&path).unwrap();
        let mut workspace = test_workspace();
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::AddCommandPreset(
                CommandPreset::new(
                    CommandPresetId::new(1),
                    name("Custom agent"),
                    "agent-cli",
                    vec!["--interactive".to_owned()],
                )
                .unwrap(),
            ),
            1,
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::AddRole(Role::with_appearance(
                role_id,
                name("Reviewer"),
                RoleColor::new("#8B5CF6").unwrap(),
                RoleIcon::new("review").unwrap(),
                content("Review for correctness"),
            )),
            2,
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::AddAgent(Agent::with_program(
                AgentId::new(1),
                name("Ada"),
                None,
                AgentProgram::Custom(CommandPresetId::new(1)),
            )),
            3,
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::AssignAgentRole {
                agent_id: AgentId::new(1),
                role_id: Some(role_id),
            },
            4,
        );
        workspace
    };

    let recovered = Journal::open(&path)
        .unwrap()
        .recover(WorkspaceId::new(7))
        .unwrap()
        .unwrap();

    assert_eq!(recovered, expected);
    assert_eq!(recovered.role(role_id).unwrap().color().as_str(), "#8B5CF6");
    assert_eq!(
        recovered.agent(AgentId::new(1)).unwrap().program(),
        AgentProgram::Custom(CommandPresetId::new(1))
    );
}

#[test]
fn chat_messages_attachments_and_unsent_drafts_survive_restart() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let thread_id = ChatThreadId::new(1);
    let attachment_id = ChatAttachmentId::new(1);
    let expected = {
        let mut journal = Journal::open(&path).unwrap();
        let mut workspace = test_workspace();
        for (id, agent_name) in [(1, "Builder"), (2, "Reviewer")] {
            persist(
                &mut journal,
                &mut workspace,
                DomainCommand::AddAgent(Agent::new(AgentId::new(id), name(agent_name), None)),
                id,
            );
        }
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::ReplaceCanvas {
                before: CanvasLayout::default(),
                after: CanvasLayout::new(
                    vec![
                        Node::new(
                            NodeId::new(1),
                            NodeTarget::Agent(AgentId::new(1)),
                            CanvasPoint::new(0.0, 0.0).unwrap(),
                            CanvasSize::new(400.0, 300.0).unwrap(),
                        ),
                        Node::new(
                            NodeId::new(2),
                            NodeTarget::Agent(AgentId::new(2)),
                            CanvasPoint::new(500.0, 0.0).unwrap(),
                            CanvasSize::new(400.0, 300.0).unwrap(),
                        ),
                    ],
                    Vec::new(),
                    vec![DomainConnection::new(
                        ConnectionId::new(1),
                        NodeId::new(1),
                        NodeId::new(2),
                        ConnectionKind::Coordination,
                    )],
                ),
            },
            3,
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::AddChatThread(ChatThread::with_color(
                thread_id,
                AgentId::new(1),
                name("Implementation"),
                ThreadColor::new("#0F766E").unwrap(),
            )),
            4,
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::AddChatAttachment(
                ChatAttachment::new(
                    attachment_id,
                    thread_id,
                    "1-notes.md",
                    "notes.md",
                    "text/markdown",
                    512,
                )
                .unwrap(),
            ),
            5,
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::UpdateChatDraft {
                thread_id,
                draft: ChatDraft::new(
                    "Please review **notes.md**",
                    vec![attachment_id],
                    vec![NodeTarget::Agent(AgentId::new(2))],
                )
                .unwrap(),
            },
            6,
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::SubmitChatDraft {
                thread_id,
                message_id: ChatMessageId::new(1),
                sent_at: timestamp(7),
            },
            7,
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::AppendAgentChatMessage(
                ChatMessage::new(
                    ChatMessageId::new(2),
                    thread_id,
                    ChatAuthor::Agent(AgentId::new(1)),
                    content("Reviewed the table and code block."),
                    Vec::new(),
                    Vec::new(),
                    timestamp(8),
                )
                .unwrap(),
            ),
            8,
        );
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::UpdateChatDraft {
                thread_id,
                draft: ChatDraft::new("Unsent follow-up", Vec::new(), Vec::new()).unwrap(),
            },
            9,
        );
        workspace
    };

    let recovered = Journal::open(&path)
        .unwrap()
        .recover(WorkspaceId::new(7))
        .unwrap()
        .unwrap();

    assert_eq!(recovered, expected);
    let thread = recovered.chat_thread(thread_id).unwrap();
    assert_eq!(thread.messages().len(), 2);
    assert_eq!(thread.draft().text(), "Unsent follow-up");
    assert_eq!(
        recovered
            .chat_attachment(attachment_id)
            .unwrap()
            .display_name(),
        "notes.md"
    );
}

#[test]
fn portable_roles_round_trip_with_a_fresh_destination_id() {
    let source = Role::with_appearance(
        RoleId::new(1),
        name("Reviewer"),
        RoleColor::new("#8B5CF6").unwrap(),
        RoleIcon::new("review").unwrap(),
        content("Review changes for regressions"),
    );

    let payload = export_role(&source).unwrap();
    let imported = import_role(&payload, RoleId::new(99)).unwrap();

    assert!(!payload.contains("\"id\""));
    assert_eq!(imported.id(), RoleId::new(99));
    assert_eq!(imported.name(), source.name());
    assert_eq!(imported.color(), source.color());
    assert_eq!(imported.icon(), source.icon());
    assert_eq!(imported.instructions(), source.instructions());
}

#[test]
fn portable_roles_reject_unknown_versions_and_invalid_fields() {
    let version_error = import_role(
        r##"{"format":"openpodium-role","version":2,"role":{"name":"Reviewer","color":"#8B5CF6","icon":"review","instructions":"Review changes"}}"##,
        RoleId::new(1),
    )
    .unwrap_err();
    assert!(matches!(
        version_error,
        RoleTransferError::UnsupportedVersion {
            found: 2,
            supported: 1
        }
    ));

    let role_error = import_role(
        r#"{"format":"openpodium-role","version":1,"role":{"name":"Reviewer","color":"violet","icon":"review","instructions":"Review changes"}}"#,
        RoleId::new(1),
    )
    .unwrap_err();
    assert!(matches!(role_error, RoleTransferError::InvalidRole(_)));
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
            supported: EVENT_FORMAT_VERSION,
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
fn version_one_event_and_snapshot_formats_remain_decodable() {
    let event = decode_event(
        br#"{
            "type":"role_added",
            "role":{
                "id":1,
                "name":"Builder",
                "instructions":"Implement the requested change"
            }
        }"#,
        1,
        1,
    )
    .unwrap();
    let workspace = decode_workspace(
        br#"{
            "id":7,
            "name":"Legacy workspace",
            "roles":[],
            "agents":[],
            "tasks":[],
            "handoffs":[],
            "nodes":[]
        }"#,
        1,
        1,
    )
    .unwrap();

    assert_eq!(event, DomainEvent::RoleAdded(test_role()));
    assert_eq!(workspace.name(), "Legacy workspace");
    assert!(workspace.settings().working_directory().is_none());
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
            supported: 2,
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
fn version_one_migration_backfills_workspace_registry_and_active_selection() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let workspace_id = WorkspaceId::new(7);
    {
        let mut journal = Journal::open(&path).unwrap();
        let mut workspace = test_workspace();
        persist(
            &mut journal,
            &mut workspace,
            DomainCommand::AddRole(test_role()),
            42,
        );
        journal
            .connection()
            .execute_batch(
                "DROP TABLE application_state;
                 DROP TABLE workspace_registry;
                 PRAGMA user_version = 1;",
            )
            .unwrap();
    }

    let journal = Journal::open(&path).unwrap();

    assert_eq!(journal.recent_workspace_ids().unwrap(), vec![workspace_id]);
    assert_eq!(journal.active_workspace_id().unwrap(), Some(workspace_id));
    assert!(journal.recover(workspace_id).unwrap().is_some());
    assert!(
        path.with_file_name("journal.sqlite.backup-v1-before-v2")
            .exists()
    );
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
            to: 2,
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
    database.with_file_name("journal.sqlite.backup-v0-before-v2")
}
