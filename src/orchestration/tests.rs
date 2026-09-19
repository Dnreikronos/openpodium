use std::fs;

use tempfile::TempDir;

use super::*;
use crate::domain::{
    Agent, AgentId, AgentProgram, DomainCommand, HandoffMessageId, Name, TaskId, TaskState,
    Timestamp,
};
use crate::ipc::{AcceptedMessage, HandoffKind, MessageId, ProtocolCommand, ResponseStatus};
use crate::workspaces::WorkspaceManager;

fn timestamp(value: u64) -> Timestamp {
    Timestamp::from_unix_millis(value)
}

fn message_id(value: &str) -> MessageId {
    MessageId::new(value).unwrap()
}

fn manager(program: AgentProgram) -> (TempDir, WorkspaceManager, crate::domain::WorkspaceId) {
    let temp = TempDir::new().unwrap();
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    let mut manager = WorkspaceManager::open(temp.path().join("openpodium.sqlite")).unwrap();
    let workspace_id = manager.create_workspace(&project, timestamp(1)).unwrap();
    for (id, name, program) in [(1, "Lead", AgentProgram::Codex), (2, "Builder", program)] {
        manager
            .execute(
                workspace_id,
                DomainCommand::AddAgent(Agent::with_program(
                    AgentId::new(id),
                    Name::new(name).unwrap(),
                    None,
                    program,
                )),
                timestamp(id + 1),
            )
            .unwrap();
    }
    (temp, manager, workspace_id)
}

fn task(message: &str, title: &str) -> AcceptedMessage {
    AcceptedMessage {
        workspace_id: 1,
        sender_agent_id: 1,
        recipient_agent_id: 2,
        command: ProtocolCommand::SendHandoff {
            message_id: message_id(message),
            recipient_agent_id: 2,
            kind: HandoffKind::Task,
            title: Some(title.to_owned()),
            body: "Implement the change".to_owned(),
            parent_message_id: None,
            response_timeout_ms: None,
        },
    }
}

#[test]
fn task_progress_and_response_update_state_and_preserve_mailbox_order() {
    let (_temp, mut manager, workspace_id) = manager(AgentProgram::Claude);
    let mut orchestrator = Orchestrator::recover(&mut manager, timestamp(10)).unwrap();
    orchestrator
        .accept(&mut manager, &task("task-1", "First"), timestamp(20))
        .unwrap();
    orchestrator
        .accept(&mut manager, &task("task-2", "Second"), timestamp(21))
        .unwrap();

    let first = orchestrator
        .prepare_next(&mut manager, timestamp(22))
        .unwrap()
        .unwrap();
    assert!(first.prompt().contains("task-1"));
    assert_eq!(first.recipient_agent_id(), AgentId::new(2));
    assert_eq!(
        first.mechanism(),
        crate::domain::DeliveryMechanism::ClaudeTerminal
    );
    orchestrator
        .finish_delivery(&mut manager, &first, Ok(()), timestamp(23))
        .unwrap();
    assert_eq!(
        manager
            .workspace(workspace_id)
            .unwrap()
            .task(TaskId::new(1))
            .unwrap()
            .state(),
        TaskState::Delivered
    );

    let second = orchestrator
        .prepare_next(&mut manager, timestamp(24))
        .unwrap()
        .unwrap();
    assert!(second.prompt().contains("task-2"));
    orchestrator
        .finish_delivery(&mut manager, &second, Ok(()), timestamp(25))
        .unwrap();

    let progress = AcceptedMessage {
        workspace_id: workspace_id.get(),
        sender_agent_id: 2,
        recipient_agent_id: 1,
        command: ProtocolCommand::ReportHandoffProgress {
            message_id: message_id("progress-1"),
            handoff_message_id: message_id("task-1"),
            body: "Tests are running".to_owned(),
        },
    };
    orchestrator
        .accept(&mut manager, &progress, timestamp(30))
        .unwrap();
    assert_eq!(
        manager
            .workspace(workspace_id)
            .unwrap()
            .task(TaskId::new(1))
            .unwrap()
            .state(),
        TaskState::Running
    );

    let response = AcceptedMessage {
        workspace_id: workspace_id.get(),
        sender_agent_id: 2,
        recipient_agent_id: 1,
        command: ProtocolCommand::RespondToHandoff {
            message_id: message_id("response-1"),
            handoff_message_id: message_id("task-1"),
            status: ResponseStatus::Completed,
            body: "Done".to_owned(),
        },
    };
    orchestrator
        .accept(&mut manager, &response, timestamp(31))
        .unwrap();
    assert_eq!(
        manager
            .workspace(workspace_id)
            .unwrap()
            .task(TaskId::new(1))
            .unwrap()
            .state(),
        TaskState::Completed
    );

    let progress_delivery = orchestrator
        .prepare_next(&mut manager, timestamp(32))
        .unwrap()
        .unwrap();
    assert!(progress_delivery.prompt().contains("Tests are running"));
    orchestrator
        .finish_delivery(&mut manager, &progress_delivery, Ok(()), timestamp(33))
        .unwrap();
    let response_delivery = orchestrator
        .prepare_next(&mut manager, timestamp(34))
        .unwrap()
        .unwrap();
    assert!(response_delivery.prompt().contains("Done"));
}

#[test]
fn retries_are_bounded_and_recovery_finishes_interrupted_attempts() {
    let (_temp, mut manager, workspace_id) = manager(AgentProgram::OpenCode);
    let mut orchestrator = Orchestrator::recover(&mut manager, timestamp(10)).unwrap();
    orchestrator
        .accept(&mut manager, &task("task-1", "Retry"), timestamp(100))
        .unwrap();
    let request = orchestrator
        .prepare_next(&mut manager, timestamp(101))
        .unwrap()
        .unwrap();
    orchestrator
        .finish_delivery(
            &mut manager,
            &request,
            Err("terminal is starting".to_owned()),
            timestamp(102),
        )
        .unwrap();
    assert!(
        orchestrator
            .prepare_next(&mut manager, timestamp(200))
            .unwrap()
            .is_none()
    );
    let retry = orchestrator
        .prepare_next(&mut manager, timestamp(352))
        .unwrap()
        .unwrap();
    assert_eq!(retry.handoff_id(), request.handoff_id());

    drop(orchestrator);
    let mut recovered = Orchestrator::recover(&mut manager, timestamp(400)).unwrap();
    let handoff = manager
        .workspace(workspace_id)
        .unwrap()
        .handoffs()
        .find(|handoff| handoff.message_id() == Some(&HandoffMessageId::new("task-1").unwrap()))
        .unwrap();
    assert!(matches!(
        handoff.delivery_attempts().last().unwrap().outcome(),
        crate::domain::DeliveryOutcome::Failed {
            retryable: true,
            ..
        }
    ));
    assert!(
        recovered
            .prepare_next(&mut manager, timestamp(401))
            .unwrap()
            .is_some()
    );
}

#[test]
fn delivery_window_exhaustion_records_failure_without_claiming_delivery() {
    let (_temp, mut manager, workspace_id) = manager(AgentProgram::Codex);
    let mut orchestrator = Orchestrator::recover(&mut manager, timestamp(10)).unwrap();
    orchestrator
        .accept(&mut manager, &task("task-1", "Expire"), timestamp(100))
        .unwrap();

    assert!(
        orchestrator
            .prepare_next(&mut manager, timestamp(30_100))
            .unwrap()
            .is_none()
    );

    let workspace = manager.workspace(workspace_id).unwrap();
    assert_eq!(
        workspace.task(TaskId::new(1)).unwrap().state(),
        TaskState::Failed
    );
    let handoff = workspace.handoff(crate::domain::HandoffId::new(1)).unwrap();
    assert!(!handoff.is_delivered());
    assert!(matches!(
        handoff.delivery_attempts().last().unwrap().outcome(),
        crate::domain::DeliveryOutcome::Failed {
            retryable: false,
            ..
        }
    ));
}

#[test]
fn unsafe_adapters_are_rejected_before_durable_work_is_created() {
    let (_temp, mut manager, workspace_id) = manager(AgentProgram::Shell);
    let mut orchestrator = Orchestrator::recover(&mut manager, timestamp(10)).unwrap();

    let error = orchestrator
        .accept(&mut manager, &task("task-1", "Unsafe"), timestamp(20))
        .unwrap_err();

    assert!(matches!(error, OrchestrationError::UnsafeAdapter { .. }));
    assert!(
        manager
            .workspace(workspace_id)
            .unwrap()
            .task(TaskId::new(1))
            .is_none()
    );
}

#[test]
fn explicit_response_deadlines_fail_unfinished_tasks() {
    let (_temp, mut manager, workspace_id) = manager(AgentProgram::Codex);
    let mut orchestrator = Orchestrator::recover(&mut manager, timestamp(10)).unwrap();
    let mut message = task("task-1", "Timeout");
    let ProtocolCommand::SendHandoff {
        response_timeout_ms,
        ..
    } = &mut message.command
    else {
        unreachable!()
    };
    *response_timeout_ms = Some(100);
    orchestrator
        .accept(&mut manager, &message, timestamp(20))
        .unwrap();

    orchestrator
        .expire_due(&mut manager, timestamp(120))
        .unwrap();

    assert!(
        orchestrator
            .prepare_next(&mut manager, timestamp(121))
            .unwrap()
            .is_none()
    );

    assert_eq!(
        manager
            .workspace(workspace_id)
            .unwrap()
            .task(TaskId::new(1))
            .unwrap()
            .state(),
        TaskState::Failed
    );
}

#[test]
fn cancellation_discards_an_undelivered_task_and_delivers_only_the_cancellation() {
    let (_temp, mut manager, workspace_id) = manager(AgentProgram::Codex);
    let mut orchestrator = Orchestrator::recover(&mut manager, timestamp(10)).unwrap();
    orchestrator
        .accept(&mut manager, &task("task-1", "Cancel me"), timestamp(20))
        .unwrap();
    let cancellation = AcceptedMessage {
        workspace_id: workspace_id.get(),
        sender_agent_id: 1,
        recipient_agent_id: 2,
        command: ProtocolCommand::CancelHandoff {
            message_id: message_id("cancel-1"),
            handoff_message_id: message_id("task-1"),
            reason: "No longer needed".to_owned(),
        },
    };
    orchestrator
        .accept(&mut manager, &cancellation, timestamp(21))
        .unwrap();

    let delivery = orchestrator
        .prepare_next(&mut manager, timestamp(22))
        .unwrap()
        .unwrap();
    assert!(delivery.prompt().contains("cancellation"));
    assert!(!delivery.prompt().contains("Title: Cancel me"));
    assert_eq!(
        manager
            .workspace(workspace_id)
            .unwrap()
            .task(TaskId::new(1))
            .unwrap()
            .state(),
        TaskState::Cancelled
    );
}
