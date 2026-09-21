use std::collections::BTreeMap;
use std::fs;

use tempfile::TempDir;

use super::*;
use crate::domain::{
    Agent, AgentId, AgentProgram, Content, DomainCommand, HandoffMessageId, HandoffResponse,
    HandoffResponseStatus, Name, Routine, RoutineApproval, RoutineApprovalDecision,
    RoutineBindingSource, RoutineCheckoutClaim, RoutineId, RoutineInputDeclaration,
    RoutineInputKey, RoutineOutputKey, RoutineResourceKey, RoutineRetryPolicy, RoutineRunId,
    RoutineRunState, RoutineStep, RoutineStepClaims, RoutineStepId, RoutineStepState, RoutineValue,
    RoutineVersion, RoutineVersionId, TaskState, Timestamp, WorkspaceId,
};
use crate::orchestration::Orchestrator;
use crate::workspaces::WorkspaceManager;

fn timestamp(value: u64) -> Timestamp {
    Timestamp::from_unix_millis(value)
}

fn name(value: &str) -> Name {
    Name::new(value).unwrap()
}

fn content(value: &str) -> Content {
    Content::new(value).unwrap()
}

fn input_key(value: &str) -> RoutineInputKey {
    RoutineInputKey::new(value).unwrap()
}

fn output_key(value: &str) -> RoutineOutputKey {
    RoutineOutputKey::new(value).unwrap()
}

fn value(text: &str) -> RoutineValue {
    RoutineValue::new(text).unwrap()
}

/// Two Codex agents on a real temporary checkout, so path canonicalization and
/// delivery mechanism selection run the same way they do in the application.
fn manager() -> (TempDir, WorkspaceManager, WorkspaceId) {
    let temp = TempDir::new().unwrap();
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    let mut manager = WorkspaceManager::open(temp.path().join("openpodium.sqlite")).unwrap();
    let workspace_id = manager.create_workspace(&project, timestamp(1)).unwrap();
    for id in 1..=2 {
        manager
            .execute(
                workspace_id,
                DomainCommand::AddAgent(Agent::with_program(
                    AgentId::new(id),
                    name(&format!("Agent {id}")),
                    None,
                    AgentProgram::Codex,
                )),
                timestamp(id + 1),
            )
            .unwrap();
    }
    (temp, manager, workspace_id)
}

fn step(
    id: u64,
    agent: u64,
    prompt: &str,
    depends_on: Vec<u64>,
    bindings: Vec<(&str, RoutineBindingSource)>,
    outputs: Vec<&str>,
) -> RoutineStep {
    RoutineStep::new(
        RoutineStepId::new(id),
        name(&format!("Step {id}")),
        AgentId::new(agent),
        content(prompt),
        depends_on.into_iter().map(RoutineStepId::new),
        bindings
            .into_iter()
            .map(|(key, source)| (input_key(key), source)),
        outputs.into_iter().map(output_key),
        RoutineApproval::NotRequired,
        RoutineRetryPolicy::default(),
        RoutineStepClaims::default(),
    )
    .unwrap()
}

fn version(steps: Vec<RoutineStep>, inputs: Vec<RoutineInputDeclaration>) -> RoutineVersion {
    RoutineVersion::new(
        RoutineVersionId::new(1),
        RoutineId::new(1),
        1,
        inputs,
        steps,
        None,
        timestamp(10),
    )
    .unwrap()
}

fn install(
    manager: &mut WorkspaceManager,
    workspace_id: WorkspaceId,
    version: RoutineVersion,
) -> RoutineId {
    let routine = Routine::new(RoutineId::new(1), name("Nightly"), None, version).unwrap();
    let routine_id = routine.id();
    manager
        .execute(
            workspace_id,
            DomainCommand::AddRoutine(routine),
            timestamp(11),
        )
        .unwrap();
    routine_id
}

/// Marks the step's handoff completed with the outputs an agent would return
/// over typed IPC.
fn respond(
    manager: &mut WorkspaceManager,
    workspace_id: WorkspaceId,
    run_id: RoutineRunId,
    step_id: RoutineStepId,
    status: HandoffResponseStatus,
    outputs: &[(&str, &str)],
    at: u64,
) {
    let workspace = manager.workspace(workspace_id).unwrap();
    let run = workspace.routine_run(run_id).unwrap();
    let attempt = run.step(step_id).unwrap().attempts().last().unwrap();
    let before = workspace.handoff(attempt.handoff_id()).unwrap().clone();
    let message_id = before.message_id().unwrap().clone();

    // Deliver first: the domain rejects a response to work that never arrived.
    let mut delivered = before.clone();
    let ordinal = delivered
        .begin_delivery(
            message_id.clone(),
            crate::domain::DeliveryMechanism::CodexTerminal,
            timestamp(at),
        )
        .unwrap();
    manager
        .execute(
            workspace_id,
            DomainCommand::UpdateHandoff {
                before: before.clone(),
                after: delivered.clone(),
            },
            timestamp(at),
        )
        .unwrap();
    let before = delivered.clone();
    delivered.complete_delivery(ordinal, timestamp(at)).unwrap();
    manager
        .execute(
            workspace_id,
            DomainCommand::UpdateHandoff {
                before,
                after: delivered.clone(),
            },
            timestamp(at),
        )
        .unwrap();

    let before = delivered.clone();
    let mut after = delivered;
    let response = HandoffResponse::new(
        HandoffMessageId::new(format!("{}-reply", message_id.as_str())).unwrap(),
        status,
        content("done"),
        timestamp(at + 1),
    )
    .with_outputs(
        outputs
            .iter()
            .map(|(key, text)| (output_key(key), value(text)))
            .collect(),
    );
    after.respond(response).unwrap();
    manager
        .execute(
            workspace_id,
            DomainCommand::UpdateHandoff { before, after },
            timestamp(at + 1),
        )
        .unwrap();
}

#[test]
fn a_run_pins_the_version_it_started_with() {
    let (_temp, mut manager, workspace_id) = manager();
    let routine_id = install(
        &mut manager,
        workspace_id,
        version(
            vec![step(
                1,
                1,
                "Build {{target}}",
                vec![],
                vec![("target", RoutineBindingSource::Input(input_key("target")))],
                vec![],
            )],
            vec![RoutineInputDeclaration::new(
                input_key("target"),
                name("Target"),
                true,
                None,
            )],
        ),
    );
    let mut scheduler = RoutineScheduler::default();
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest {
                inputs: BTreeMap::from([(input_key("target"), value("release"))]),
                ..RunRequest::default()
            },
            timestamp(20),
        )
        .unwrap();

    // Editing the routine afterwards must not change what the run executes.
    let second = RoutineVersion::new(
        RoutineVersionId::new(2),
        RoutineId::new(1),
        2,
        Vec::new(),
        vec![step(
            1,
            2,
            "A completely different prompt",
            vec![],
            vec![],
            vec![],
        )],
        None,
        timestamp(21),
    )
    .unwrap();
    manager
        .execute(
            workspace_id,
            DomainCommand::AddRoutineVersion {
                routine_id,
                version: second,
            },
            timestamp(21),
        )
        .unwrap();

    let run = manager
        .workspace(workspace_id)
        .unwrap()
        .routine_run(run_id)
        .unwrap();
    assert_eq!(run.version_id(), RoutineVersionId::new(1));
    assert_eq!(run.pin().version().number(), 1);
    assert_eq!(
        run.pin().inputs().get(&input_key("target")),
        Some(&value("release"))
    );
    assert_eq!(
        run.pin().agent(RoutineStepId::new(1)),
        Some(AgentId::new(1))
    );
    assert_eq!(
        manager
            .workspace(workspace_id)
            .unwrap()
            .routine(routine_id)
            .unwrap()
            .latest_version()
            .number(),
        2
    );
}

#[test]
fn sequential_steps_wait_for_their_dependency_and_bind_its_output() {
    let (_temp, mut manager, workspace_id) = manager();
    let routine_id = install(
        &mut manager,
        workspace_id,
        version(
            vec![
                step(1, 1, "Investigate", vec![], vec![], vec!["finding"]),
                step(
                    2,
                    2,
                    "Fix {{report}}",
                    vec![1],
                    vec![(
                        "report",
                        RoutineBindingSource::StepOutput {
                            step_id: RoutineStepId::new(1),
                            key: output_key("finding"),
                        },
                    )],
                    vec![],
                ),
            ],
            Vec::new(),
        ),
    );
    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();

    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(21))
        .unwrap();
    let run = manager
        .workspace(workspace_id)
        .unwrap()
        .routine_run(run_id)
        .unwrap();
    assert_eq!(
        run.step(RoutineStepId::new(1)).unwrap().state(),
        RoutineStepState::Dispatched
    );
    assert_eq!(
        run.step(RoutineStepId::new(2)).unwrap().state(),
        RoutineStepState::Pending,
        "the dependent step must not start before its dependency completes"
    );

    respond(
        &mut manager,
        workspace_id,
        run_id,
        RoutineStepId::new(1),
        HandoffResponseStatus::Completed,
        &[("finding", "the parser drops escapes")],
        30,
    );
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(40))
        .unwrap();

    let workspace = manager.workspace(workspace_id).unwrap();
    let run = workspace.routine_run(run_id).unwrap();
    assert_eq!(
        run.step(RoutineStepId::new(1)).unwrap().state(),
        RoutineStepState::Completed
    );
    let second = run.step(RoutineStepId::new(2)).unwrap();
    assert_eq!(second.state(), RoutineStepState::Dispatched);
    let task_id = second.attempts().last().unwrap().task_id();
    assert_eq!(
        workspace.task(task_id).unwrap().prompt().as_str(),
        "Fix the parser drops escapes",
        "the dependent step's prompt must use the recorded output"
    );
}

#[test]
fn completing_without_a_declared_output_fails_the_step() {
    let (_temp, mut manager, workspace_id) = manager();
    let routine_id = install(
        &mut manager,
        workspace_id,
        version(
            vec![step(1, 1, "Investigate", vec![], vec![], vec!["finding"])],
            Vec::new(),
        ),
    );
    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(21))
        .unwrap();
    respond(
        &mut manager,
        workspace_id,
        run_id,
        RoutineStepId::new(1),
        HandoffResponseStatus::Completed,
        &[],
        30,
    );
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(40))
        .unwrap();

    let workspace = manager.workspace(workspace_id).unwrap();
    let run = workspace.routine_run(run_id).unwrap();
    let step = run.step(RoutineStepId::new(1)).unwrap();
    assert_eq!(step.state(), RoutineStepState::Failed);
    assert!(
        step.failure().unwrap().as_str().contains("finding"),
        "the failure must name the missing output, not invent a value"
    );
    assert_eq!(run.state(), RoutineRunState::Failed);
}

#[test]
fn parallel_steps_sharing_a_checkout_do_not_run_together() {
    let (_temp, mut manager, workspace_id) = manager();
    // Both steps declare the same named resource and different agents, so only
    // the resource claim can keep them apart.
    let claims = RoutineStepClaims::new(
        RoutineCheckoutClaim::AgentDefault,
        [RoutineResourceKey::new("database").unwrap()],
    )
    .unwrap();
    let steps = vec![
        RoutineStep::new(
            RoutineStepId::new(1),
            name("Migrate"),
            AgentId::new(1),
            content("Migrate"),
            [],
            [],
            [],
            RoutineApproval::NotRequired,
            RoutineRetryPolicy::default(),
            claims.clone(),
        )
        .unwrap(),
        RoutineStep::new(
            RoutineStepId::new(2),
            name("Seed"),
            AgentId::new(2),
            content("Seed"),
            [],
            [],
            [],
            RoutineApproval::NotRequired,
            RoutineRetryPolicy::default(),
            claims,
        )
        .unwrap(),
    ];
    let routine_id = install(&mut manager, workspace_id, version(steps, Vec::new()));
    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(21))
        .unwrap();

    let run = manager
        .workspace(workspace_id)
        .unwrap()
        .routine_run(run_id)
        .unwrap();
    let dispatched = run
        .steps()
        .filter(|step| step.state() == RoutineStepState::Dispatched)
        .count();
    assert_eq!(
        dispatched, 1,
        "two steps claiming the same resource must not run at once"
    );

    respond(
        &mut manager,
        workspace_id,
        run_id,
        RoutineStepId::new(1),
        HandoffResponseStatus::Completed,
        &[],
        30,
    );
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(40))
        .unwrap();
    let run = manager
        .workspace(workspace_id)
        .unwrap()
        .routine_run(run_id)
        .unwrap();
    assert_eq!(
        run.step(RoutineStepId::new(2)).unwrap().state(),
        RoutineStepState::Dispatched,
        "the second step runs once the claim is released"
    );
}

#[test]
fn a_dispatched_step_is_interrupted_after_a_restart() {
    let (temp, mut manager, workspace_id) = manager();
    let routine_id = install(
        &mut manager,
        workspace_id,
        version(
            vec![step(1, 1, "Investigate", vec![], vec![], vec![])],
            Vec::new(),
        ),
    );
    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(21))
        .unwrap();
    let task_id = manager
        .workspace(workspace_id)
        .unwrap()
        .routine_run(run_id)
        .unwrap()
        .step(RoutineStepId::new(1))
        .unwrap()
        .attempts()
        .last()
        .unwrap()
        .task_id();
    drop(manager);

    let mut reopened = WorkspaceManager::open(temp.path().join("openpodium.sqlite")).unwrap();
    let recovered = RoutineScheduler::recover(&mut reopened, timestamp(50)).unwrap();
    let run = reopened
        .workspace(workspace_id)
        .unwrap()
        .routine_run(run_id)
        .unwrap();
    let step = run.step(RoutineStepId::new(1)).unwrap();
    assert_eq!(step.state(), RoutineStepState::Interrupted);
    assert!(step.interruption().is_some());
    assert_eq!(
        run.held_reservations().len(),
        2,
        "an interrupted step keeps its agent and checkout until a user resolves it"
    );
    assert_eq!(
        reopened
            .workspace(workspace_id)
            .unwrap()
            .task(task_id)
            .unwrap()
            .state(),
        TaskState::Queued,
        "the task lifecycle stays authoritative and is untouched by recovery"
    );
    assert_eq!(recovered.pending_registration_count(), 1);
}

#[test]
fn confirming_a_cancelled_step_settles_the_run() {
    let (_temp, mut manager, workspace_id) = manager();
    let retried = RoutineStep::new(
        RoutineStepId::new(1),
        name("Flaky"),
        AgentId::new(1),
        content("Try"),
        [],
        [],
        [],
        RoutineApproval::NotRequired,
        RoutineRetryPolicy::new(2).unwrap(),
        RoutineStepClaims::default(),
    )
    .unwrap();
    let routine_id = install(
        &mut manager,
        workspace_id,
        version(vec![retried], Vec::new()),
    );
    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(21))
        .unwrap();
    let first_task = manager
        .workspace(workspace_id)
        .unwrap()
        .routine_run(run_id)
        .unwrap()
        .step(RoutineStepId::new(1))
        .unwrap()
        .attempts()
        .last()
        .unwrap()
        .task_id();

    scheduler
        .cancel_run(
            &mut manager,
            &mut orchestrator,
            workspace_id,
            run_id,
            timestamp(30),
        )
        .unwrap();
    assert_eq!(
        manager
            .workspace(workspace_id)
            .unwrap()
            .routine_run(run_id)
            .unwrap()
            .step(RoutineStepId::new(1))
            .unwrap()
            .state(),
        RoutineStepState::Interrupted,
        "a cancelled handoff alone does not prove the agent stopped"
    );

    scheduler
        .resolve_interruption(
            &mut manager,
            workspace_id,
            run_id,
            RoutineStepId::new(1),
            timestamp(40),
        )
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(41))
        .unwrap();

    let run = manager
        .workspace(workspace_id)
        .unwrap()
        .routine_run(run_id)
        .unwrap();
    let step = run.step(RoutineStepId::new(1)).unwrap();
    assert_eq!(
        step.attempts().len(),
        1,
        "the confirmed work is not retried"
    );
    assert_eq!(step.attempts()[0].task_id(), first_task);
    assert_eq!(step.state(), RoutineStepState::Cancelled);
    assert_eq!(
        run.state(),
        RoutineRunState::Cancelled,
        "a cancelled run settles instead of staying active forever"
    );
    assert!(
        run.held_reservations().is_empty(),
        "a settled run releases the claims it held"
    );

    // A settled run no longer blocks the next one.
    scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(50),
        )
        .unwrap();
}

#[test]
fn an_approval_gate_holds_the_step_until_a_decision() {
    let (_temp, mut manager, workspace_id) = manager();
    let gated = RoutineStep::new(
        RoutineStepId::new(1),
        name("Deploy"),
        AgentId::new(1),
        content("Deploy"),
        [],
        [],
        [],
        RoutineApproval::Required,
        RoutineRetryPolicy::default(),
        RoutineStepClaims::default(),
    )
    .unwrap();
    let routine_id = install(&mut manager, workspace_id, version(vec![gated], Vec::new()));
    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(21))
        .unwrap();
    assert_eq!(
        manager
            .workspace(workspace_id)
            .unwrap()
            .routine_run(run_id)
            .unwrap()
            .step(RoutineStepId::new(1))
            .unwrap()
            .state(),
        RoutineStepState::AwaitingApproval
    );

    scheduler
        .decide_approval(
            &mut manager,
            workspace_id,
            run_id,
            RoutineStepId::new(1),
            RoutineApprovalDecision::Approved,
            None,
            timestamp(25),
        )
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(26))
        .unwrap();
    assert_eq!(
        manager
            .workspace(workspace_id)
            .unwrap()
            .routine_run(run_id)
            .unwrap()
            .step(RoutineStepId::new(1))
            .unwrap()
            .state(),
        RoutineStepState::Dispatched
    );
}

#[test]
fn a_second_run_of_the_same_routine_is_refused_while_one_is_active() {
    let (_temp, mut manager, workspace_id) = manager();
    let routine_id = install(
        &mut manager,
        workspace_id,
        version(
            vec![step(1, 1, "Investigate", vec![], vec![], vec![])],
            Vec::new(),
        ),
    );
    let mut scheduler = RoutineScheduler::default();
    scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();
    let second = scheduler.start_run(
        &mut manager,
        workspace_id,
        routine_id,
        RunRequest::default(),
        timestamp(21),
    );
    assert!(matches!(
        second,
        Err(RoutineSchedulerError::RunAlreadyActive { .. })
    ));
}

// Trigger adapters.

use crate::domain::{
    RoutineCadence, RoutineSchedule, RoutineTrigger, RoutineTriggerId, RoutineTriggerKind,
};

fn install_trigger(
    manager: &mut WorkspaceManager,
    workspace_id: WorkspaceId,
    routine_id: RoutineId,
    kind: RoutineTriggerKind,
    enabled: bool,
) -> RoutineTriggerId {
    let trigger = RoutineTrigger::new(
        RoutineTriggerId::new(1),
        routine_id,
        name("Trigger"),
        kind,
        enabled,
        BTreeMap::new(),
    )
    .unwrap();
    let trigger_id = trigger.id();
    manager
        .execute(
            workspace_id,
            DomainCommand::PutRoutineTrigger {
                routine_id,
                trigger,
            },
            timestamp(12),
        )
        .unwrap();
    trigger_id
}

fn simple_routine(manager: &mut WorkspaceManager, workspace_id: WorkspaceId) -> RoutineId {
    install(
        manager,
        workspace_id,
        version(
            vec![step(1, 1, "Investigate", vec![], vec![], vec![])],
            Vec::new(),
        ),
    )
}

#[test]
fn a_filesystem_trigger_debounces_and_deduplicates_what_it_observed() {
    let (temp, mut manager, workspace_id) = manager();
    let routine_id = simple_routine(&mut manager, workspace_id);
    install_trigger(
        &mut manager,
        workspace_id,
        routine_id,
        RoutineTriggerKind::Filesystem {
            patterns: vec!["**/*.rs".to_owned()],
            debounce_ms: 100,
        },
        true,
    );
    let project = temp.path().join("project");
    let mut watcher = TriggerWatcher::new(timestamp(1_000));

    // The first observation is a baseline, not a change.
    assert!(watcher.poll(&manager, timestamp(1_000)).events.is_empty());

    fs::write(project.join("main.rs"), "fn main() {}").unwrap();
    assert!(
        watcher.poll(&manager, timestamp(1_010)).events.is_empty(),
        "a change inside the scan interval is not observed yet"
    );
    assert!(
        watcher.poll(&manager, timestamp(2_000)).events.is_empty(),
        "the first scan that observes a change starts its debounce window"
    );
    let poll = watcher.poll(&manager, timestamp(3_000));
    assert_eq!(poll.events.len(), 1);
    let occurrence = poll.events[0].firing.occurrence.clone();

    // Re-observing the same tree must not produce a second occurrence.
    watcher.mark_consumed(&poll.events[0]);
    assert!(watcher.poll(&manager, timestamp(4_000)).events.is_empty());

    fs::write(project.join("notes.txt"), "ignored").unwrap();
    assert!(
        watcher.poll(&manager, timestamp(5_000)).events.is_empty(),
        "paths outside the trigger's patterns are filtered out"
    );

    fs::write(project.join("main.rs"), "fn main() { todo!() }").unwrap();
    assert!(watcher.poll(&manager, timestamp(6_000)).events.is_empty());
    let poll = watcher.poll(&manager, timestamp(7_000));
    assert_eq!(poll.events.len(), 1);
    assert_ne!(
        poll.events[0].firing.occurrence, occurrence,
        "a new observed state is a new occurrence"
    );
}

#[test]
fn a_disabled_trigger_is_not_observed_and_cannot_start_a_run() {
    let (_temp, mut manager, workspace_id) = manager();
    let routine_id = simple_routine(&mut manager, workspace_id);
    let trigger_id = install_trigger(
        &mut manager,
        workspace_id,
        routine_id,
        RoutineTriggerKind::Schedule(
            RoutineSchedule::new(
                RoutineCadence::Hourly { minute: 0 },
                0,
                None,
                Some(timestamp(1_000)),
            )
            .unwrap(),
        ),
        false,
    );
    let mut watcher = TriggerWatcher::new(timestamp(500));
    assert!(watcher.poll(&manager, timestamp(5_000)).events.is_empty());

    let mut scheduler = RoutineScheduler::default();
    let refused = scheduler.start_run(
        &mut manager,
        workspace_id,
        routine_id,
        RunRequest {
            trigger: Some(TriggerFiring {
                trigger_id,
                occurrence: crate::domain::RoutineOccurrenceKey::new("sched-1-1000").unwrap(),
                next_occurrence: None,
            }),
            ..RunRequest::default()
        },
        timestamp(5_000),
    );
    assert!(matches!(
        refused,
        Err(RoutineSchedulerError::TriggerDisabled(_))
    ));
}

#[test]
fn schedule_occurrences_missed_while_closed_are_skipped_and_counted() {
    let (_temp, mut manager, workspace_id) = manager();
    let routine_id = simple_routine(&mut manager, workspace_id);
    let hour = 3_600_000_u64;
    let trigger_id = install_trigger(
        &mut manager,
        workspace_id,
        routine_id,
        RoutineTriggerKind::Schedule(
            RoutineSchedule::new(
                RoutineCadence::Hourly { minute: 0 },
                0,
                None,
                Some(timestamp(hour)),
            )
            .unwrap(),
        ),
        true,
    );

    // The session starts three hours after the stored occurrence, so those
    // three passed while OpenPodium was closed.
    let session_start = timestamp(hour * 4);
    let mut watcher = TriggerWatcher::new(session_start);
    let poll = watcher.poll(&manager, timestamp(hour * 4 + 60_000));

    assert_eq!(poll.missed.len(), 1);
    assert_eq!(poll.missed[0].trigger_id, trigger_id);
    assert_eq!(poll.missed[0].skipped, 3);
    assert_eq!(
        poll.events.len(),
        1,
        "the occurrence due this session fires"
    );
    assert_eq!(
        poll.events[0].firing.occurrence.as_str(),
        format!("sched-{}-{}", trigger_id.get(), hour * 4)
    );
}

#[test]
fn a_duplicate_trigger_occurrence_cannot_start_a_second_run() {
    let (_temp, mut manager, workspace_id) = manager();
    let routine_id = simple_routine(&mut manager, workspace_id);
    let trigger_id = install_trigger(
        &mut manager,
        workspace_id,
        routine_id,
        RoutineTriggerKind::Manual,
        true,
    );
    let occurrence = crate::domain::RoutineOccurrenceKey::new("manual-1").unwrap();
    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    let firing = TriggerFiring {
        trigger_id,
        occurrence: occurrence.clone(),
        next_occurrence: None,
    };
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest {
                trigger: Some(firing.clone()),
                ..RunRequest::default()
            },
            timestamp(20),
        )
        .unwrap();

    // Finish the run so the "one active run" rule is not what refuses the
    // duplicate; the consumed occurrence must be what stops it.
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(21))
        .unwrap();
    respond(
        &mut manager,
        workspace_id,
        run_id,
        RoutineStepId::new(1),
        HandoffResponseStatus::Completed,
        &[],
        30,
    );
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(40))
        .unwrap();
    assert_eq!(
        manager
            .workspace(workspace_id)
            .unwrap()
            .routine_run(run_id)
            .unwrap()
            .state(),
        RoutineRunState::Completed
    );

    let duplicate = scheduler.start_run(
        &mut manager,
        workspace_id,
        routine_id,
        RunRequest {
            trigger: Some(firing),
            ..RunRequest::default()
        },
        timestamp(50),
    );
    assert!(matches!(
        duplicate,
        Err(RoutineSchedulerError::OccurrenceAlreadyConsumed(_))
    ));
}

#[test]
fn a_routine_template_gives_each_run_its_own_agents() {
    let (_temp, mut manager, workspace_id) = manager();

    // Put the authoring agent on the canvas so it can be exported, then save a
    // template of that arrangement.
    manager
        .execute(
            workspace_id,
            DomainCommand::AddNode(crate::domain::Node::new(
                crate::domain::NodeId::new(1),
                crate::domain::NodeTarget::Agent(AgentId::new(1)),
                crate::domain::CanvasPoint::new(0.0, 0.0).unwrap(),
                crate::domain::CanvasSize::new(320.0, 200.0).unwrap(),
            )),
            timestamp(5),
        )
        .unwrap();
    let template = manager
        .export_template(workspace_id, &[crate::domain::NodeId::new(1)])
        .unwrap();

    let step = RoutineStep::new(
        RoutineStepId::new(1),
        name("Investigate"),
        AgentId::new(1),
        content("Investigate"),
        [],
        [],
        [],
        RoutineApproval::NotRequired,
        RoutineRetryPolicy::default(),
        RoutineStepClaims::default(),
    )
    .unwrap();
    let version = RoutineVersion::new(
        RoutineVersionId::new(1),
        RoutineId::new(1),
        1,
        Vec::new(),
        vec![step],
        Some(Content::new(template).unwrap()),
        timestamp(10),
    )
    .unwrap();
    let routine_id = install(&mut manager, workspace_id, version);

    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();

    let run = manager
        .workspace(workspace_id)
        .unwrap()
        .routine_run(run_id)
        .unwrap();
    let bound = run.pin().agent(RoutineStepId::new(1)).unwrap();
    assert_ne!(
        bound,
        AgentId::new(1),
        "the run binds the agent its template instantiation created"
    );
    assert!(
        manager
            .workspace(workspace_id)
            .unwrap()
            .agent(bound)
            .is_some(),
        "the instantiated agent exists in the workspace"
    );

    // The run dispatches to the agent it bound, not to the authoring one.
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(21))
        .unwrap();
    let workspace = manager.workspace(workspace_id).unwrap();
    let attempt = workspace
        .routine_run(run_id)
        .unwrap()
        .step(RoutineStepId::new(1))
        .unwrap()
        .attempts()
        .last()
        .unwrap();
    assert_eq!(attempt.agent_id(), bound);
    assert_eq!(
        workspace.task(attempt.task_id()).unwrap().assignee(),
        Some(bound)
    );
}

/// Drives the real orchestrator path an agent's completion takes: delivery of
/// the prompt, then an authenticated response routed to the scheduler.
fn complete_through_orchestrator(
    manager: &mut WorkspaceManager,
    orchestrator: &mut Orchestrator,
    workspace_id: WorkspaceId,
    run_id: RoutineRunId,
    step_id: RoutineStepId,
    outputs: &[(&str, &str)],
    at: u64,
) {
    let request = orchestrator
        .prepare_next(manager, timestamp(at))
        .unwrap()
        .expect("a dispatched routine step has a pending delivery");
    orchestrator
        .finish_delivery(manager, &request, Ok(()), timestamp(at))
        .unwrap();

    let workspace = manager.workspace(workspace_id).unwrap();
    let attempt = workspace
        .routine_run(run_id)
        .unwrap()
        .step(step_id)
        .unwrap()
        .attempts()
        .last()
        .unwrap();
    let handoff = workspace.handoff(attempt.handoff_id()).unwrap();
    let message_id = handoff.message_id().unwrap().as_str().to_owned();
    let agent_id = handoff.recipient().get();

    let accepted = crate::ipc::AcceptedMessage {
        workspace_id: workspace_id.get(),
        sender_agent_id: agent_id,
        recipient: crate::ipc::MessagePeer::Routine,
        command: crate::ipc::ProtocolCommand::RespondToHandoff {
            message_id: crate::ipc::MessageId::new(format!("{message_id}-reply")).unwrap(),
            handoff_message_id: crate::ipc::MessageId::new(message_id).unwrap(),
            status: crate::ipc::ResponseStatus::Completed,
            body: "Done".to_owned(),
            outputs: outputs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        },
    };
    orchestrator
        .accept(manager, &accepted, timestamp(at + 1))
        .unwrap();
}

#[test]
fn a_routine_step_completes_through_the_authenticated_response_path() {
    let (_temp, mut manager, workspace_id) = manager();
    let routine_id = install(
        &mut manager,
        workspace_id,
        version(
            vec![
                step(1, 1, "Investigate", vec![], vec![], vec!["finding"]),
                step(
                    2,
                    2,
                    "Fix {{report}}",
                    vec![1],
                    vec![(
                        "report",
                        RoutineBindingSource::StepOutput {
                            step_id: RoutineStepId::new(1),
                            key: output_key("finding"),
                        },
                    )],
                    vec![],
                ),
            ],
            Vec::new(),
        ),
    );
    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(21))
        .unwrap();

    complete_through_orchestrator(
        &mut manager,
        &mut orchestrator,
        workspace_id,
        run_id,
        RoutineStepId::new(1),
        &[("finding", "the parser drops escapes")],
        30,
    );
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(40))
        .unwrap();

    let workspace = manager.workspace(workspace_id).unwrap();
    let run = workspace.routine_run(run_id).unwrap();
    let first = run.step(RoutineStepId::new(1)).unwrap();
    assert_eq!(first.state(), RoutineStepState::Completed);
    assert_eq!(
        first
            .outputs()
            .get(&output_key("finding"))
            .unwrap()
            .as_str(),
        "the parser drops escapes"
    );
    assert_eq!(
        workspace
            .task(first.attempts()[0].task_id())
            .unwrap()
            .state(),
        TaskState::Completed,
        "the task lifecycle stays authoritative and follows the same response"
    );

    let second = run.step(RoutineStepId::new(2)).unwrap();
    assert_eq!(second.state(), RoutineStepState::Dispatched);
    assert_eq!(
        workspace
            .task(second.attempts().last().unwrap().task_id())
            .unwrap()
            .prompt()
            .as_str(),
        "Fix the parser drops escapes"
    );
}

#[test]
fn a_retry_after_a_live_task_still_records_its_predecessor() {
    let (_temp, mut manager, workspace_id) = manager();
    let retried = RoutineStep::new(
        RoutineStepId::new(1),
        name("Flaky"),
        AgentId::new(1),
        content("Try"),
        [],
        [],
        [],
        RoutineApproval::NotRequired,
        RoutineRetryPolicy::new(2).unwrap(),
        RoutineStepClaims::default(),
    )
    .unwrap();
    let routine_id = install(
        &mut manager,
        workspace_id,
        version(vec![retried], Vec::new()),
    );
    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(21))
        .unwrap();

    // Cancel and confirm the agent stopped. The first task never left the queue,
    // so the retry has to leave it in a state a retry source may be in.
    scheduler
        .cancel_run(
            &mut manager,
            &mut orchestrator,
            workspace_id,
            run_id,
            timestamp(30),
        )
        .unwrap();
    scheduler
        .resolve_interruption(
            &mut manager,
            workspace_id,
            run_id,
            RoutineStepId::new(1),
            timestamp(40),
        )
        .unwrap();

    let first_task = manager
        .workspace(workspace_id)
        .unwrap()
        .routine_run(run_id)
        .unwrap()
        .step(RoutineStepId::new(1))
        .unwrap()
        .attempts()[0]
        .task_id();
    assert!(
        matches!(
            manager
                .workspace(workspace_id)
                .unwrap()
                .task(first_task)
                .unwrap()
                .state(),
            TaskState::Failed | TaskState::Cancelled
        ),
        "a finished attempt must leave its task in a terminal state"
    );
}

#[test]
fn a_retry_after_a_completed_task_does_not_claim_an_invalid_predecessor() {
    let (_temp, mut manager, workspace_id) = manager();
    // The agent answers "completed" but omits the declared output, so the step
    // fails while its task legitimately completed.
    let retried = RoutineStep::new(
        RoutineStepId::new(1),
        name("Investigate"),
        AgentId::new(1),
        content("Investigate"),
        [],
        [],
        [output_key("finding")],
        RoutineApproval::NotRequired,
        RoutineRetryPolicy::new(2).unwrap(),
        RoutineStepClaims::default(),
    )
    .unwrap();
    let routine_id = install(
        &mut manager,
        workspace_id,
        version(vec![retried], Vec::new()),
    );
    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(21))
        .unwrap();
    complete_through_orchestrator(
        &mut manager,
        &mut orchestrator,
        workspace_id,
        run_id,
        RoutineStepId::new(1),
        &[],
        30,
    );

    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(40))
        .unwrap();

    let workspace = manager.workspace(workspace_id).unwrap();
    let step = workspace
        .routine_run(run_id)
        .unwrap()
        .step(RoutineStepId::new(1))
        .unwrap();
    assert_eq!(
        step.attempts().len(),
        2,
        "the step retried within its budget"
    );
    assert_eq!(step.state(), RoutineStepState::Dispatched);
    assert_eq!(
        workspace
            .task(step.attempts()[0].task_id())
            .unwrap()
            .state(),
        TaskState::Completed
    );
}

#[test]
fn two_routines_cannot_hold_the_same_checkout_at_once() {
    let (_temp, mut manager, workspace_id) = manager();
    // Both routines bind different agents, and neither declares a named
    // resource, so only the checkout they resolve to can keep them apart.
    for (routine_id, agent) in [(1_u64, 1_u64), (2, 2)] {
        let version = RoutineVersion::new(
            RoutineVersionId::new(routine_id),
            RoutineId::new(routine_id),
            1,
            Vec::new(),
            vec![
                RoutineStep::new(
                    RoutineStepId::new(1),
                    name("Build"),
                    AgentId::new(agent),
                    content("Build"),
                    [],
                    [],
                    [],
                    RoutineApproval::NotRequired,
                    RoutineRetryPolicy::default(),
                    RoutineStepClaims::default(),
                )
                .unwrap(),
            ],
            None,
            timestamp(10),
        )
        .unwrap();
        let routine = Routine::new(
            RoutineId::new(routine_id),
            name(&format!("Routine {routine_id}")),
            None,
            version,
        )
        .unwrap();
        manager
            .execute(
                workspace_id,
                DomainCommand::AddRoutine(routine),
                timestamp(10 + routine_id),
            )
            .unwrap();
    }

    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    let first = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            RoutineId::new(1),
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();
    let second = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            RoutineId::new(2),
            RunRequest::default(),
            timestamp(21),
        )
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(22))
        .unwrap();

    let workspace = manager.workspace(workspace_id).unwrap();
    let dispatched = [first, second]
        .into_iter()
        .filter(|run_id| {
            workspace
                .routine_run(*run_id)
                .unwrap()
                .step(RoutineStepId::new(1))
                .unwrap()
                .state()
                == RoutineStepState::Dispatched
        })
        .count();
    assert_eq!(
        dispatched, 1,
        "two runs on the same checkout must not execute at once"
    );

    complete_through_orchestrator(
        &mut manager,
        &mut orchestrator,
        workspace_id,
        first,
        RoutineStepId::new(1),
        &[],
        30,
    );
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(40))
        .unwrap();
    assert_eq!(
        manager
            .workspace(workspace_id)
            .unwrap()
            .routine_run(second)
            .unwrap()
            .step(RoutineStepId::new(1))
            .unwrap()
            .state(),
        RoutineStepState::Dispatched,
        "the waiting run starts once the checkout is released"
    );
}

#[test]
fn a_response_that_arrives_after_cancellation_does_not_resurrect_the_step() {
    let (_temp, mut manager, workspace_id) = manager();
    let routine_id = install(
        &mut manager,
        workspace_id,
        version(
            vec![step(1, 1, "Investigate", vec![], vec![], vec![])],
            Vec::new(),
        ),
    );
    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(21))
        .unwrap();

    // Deliver, then cancel while the agent is still working.
    let request = orchestrator
        .prepare_next(&mut manager, timestamp(22))
        .unwrap()
        .unwrap();
    orchestrator
        .finish_delivery(&mut manager, &request, Ok(()), timestamp(22))
        .unwrap();
    scheduler
        .cancel_run(
            &mut manager,
            &mut orchestrator,
            workspace_id,
            run_id,
            timestamp(30),
        )
        .unwrap();

    // The agent answers anyway. The handoff is already terminal, so the late
    // response is refused rather than silently completing a cancelled step.
    let workspace = manager.workspace(workspace_id).unwrap();
    let attempt = workspace
        .routine_run(run_id)
        .unwrap()
        .step(RoutineStepId::new(1))
        .unwrap()
        .attempts()
        .last()
        .unwrap();
    let message_id = workspace
        .handoff(attempt.handoff_id())
        .unwrap()
        .message_id()
        .unwrap()
        .as_str()
        .to_owned();
    let late = crate::ipc::AcceptedMessage {
        workspace_id: workspace_id.get(),
        sender_agent_id: 1,
        recipient: crate::ipc::MessagePeer::Routine,
        command: crate::ipc::ProtocolCommand::RespondToHandoff {
            message_id: crate::ipc::MessageId::new(format!("{message_id}-late")).unwrap(),
            handoff_message_id: crate::ipc::MessageId::new(message_id).unwrap(),
            status: crate::ipc::ResponseStatus::Completed,
            body: "Finished anyway".to_owned(),
            outputs: BTreeMap::new(),
        },
    };
    assert!(
        orchestrator
            .accept(&mut manager, &late, timestamp(35))
            .is_err(),
        "a response to a cancelled handoff is rejected"
    );

    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(40))
        .unwrap();
    let run = manager
        .workspace(workspace_id)
        .unwrap()
        .routine_run(run_id)
        .unwrap();
    assert_eq!(
        run.step(RoutineStepId::new(1)).unwrap().state(),
        RoutineStepState::Interrupted
    );
    assert_eq!(
        run.held_reservations().len(),
        2,
        "the cancelled step keeps its reservations until shutdown is confirmed"
    );
}

#[test]
fn a_git_trigger_records_the_revisions_it_observed_and_holds_its_baseline() {
    let (temp, mut manager, workspace_id) = manager();
    let project = temp.path().join("project");
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .current_dir(&project)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        matches!(status, Ok(status) if status.success())
    };
    if !git(&["init", "--initial-branch=main"]) {
        return; // Git is unavailable; the adapter has nothing to observe.
    }
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "user.name", "Test"]);
    fs::write(project.join("README.md"), "first").unwrap();
    git(&["add", "."]);
    assert!(git(&["commit", "-m", "first"]));

    // The routine declares the inputs the trigger offers, so they are recorded.
    let routine_id = install(
        &mut manager,
        workspace_id,
        version(
            vec![step(
                1,
                1,
                "Review {{after}}",
                vec![],
                vec![("after", RoutineBindingSource::Input(input_key("git.after")))],
                vec![],
            )],
            vec![
                RoutineInputDeclaration::new(input_key("git.after"), name("After"), true, None),
                RoutineInputDeclaration::new(input_key("git.before"), name("Before"), false, None),
            ],
        ),
    );
    install_trigger(
        &mut manager,
        workspace_id,
        routine_id,
        RoutineTriggerKind::Git {
            refs: vec!["main".to_owned()],
        },
        true,
    );

    let mut watcher = TriggerWatcher::new(timestamp(1_000));
    assert!(
        watcher.poll(&manager, timestamp(1_000)).events.is_empty(),
        "the first observation is a baseline, not a change"
    );

    fs::write(project.join("README.md"), "second").unwrap();
    git(&["add", "."]);
    assert!(git(&["commit", "-m", "second"]));

    assert!(
        watcher.poll(&manager, timestamp(1_100)).events.is_empty(),
        "Git polling is throttled independently of the UI tick"
    );
    let poll = watcher.poll(&manager, timestamp(2_000));
    assert_eq!(poll.events.len(), 1);
    let event = poll.events[0].clone();
    assert!(event.observed.contains_key(&input_key("git.after")));
    assert_ne!(
        event.observed.get(&input_key("git.before")),
        event.observed.get(&input_key("git.after"))
    );

    // Without consuming the event the baseline holds, so a transient failure
    // to start the run re-reports the same change rather than dropping it.
    assert_eq!(watcher.poll(&manager, timestamp(3_000)).events.len(), 1);

    watcher.mark_consumed(&event);
    assert!(watcher.poll(&manager, timestamp(4_000)).events.is_empty());

    let mut scheduler = RoutineScheduler::default();
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest {
                observed: event.observed.clone(),
                trigger: Some(event.firing.clone()),
                ..RunRequest::default()
            },
            timestamp(1_400),
        )
        .unwrap();
    let run = manager
        .workspace(workspace_id)
        .unwrap()
        .routine_run(run_id)
        .unwrap();
    assert_eq!(
        run.pin().inputs().get(&input_key("git.after")),
        event.observed.get(&input_key("git.after")),
        "the run records the revision the trigger observed"
    );
}

// Regression cover for the ways a run could lose track of live work.
#[test]
fn a_restart_does_not_paste_an_interrupted_prompt_again() {
    let (temp, mut manager, workspace_id) = manager();
    let routine_id = simple_routine(&mut manager, workspace_id);
    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(21))
        .unwrap();

    // Crash partway through pasting the prompt: the attempt is recorded as
    // started and nobody can say whether the agent received it.
    let _request = orchestrator
        .prepare_next(&mut manager, timestamp(22))
        .unwrap()
        .unwrap();
    drop(manager);

    let mut reopened = WorkspaceManager::open(temp.path().join("openpodium.sqlite")).unwrap();
    let mut recovered_orchestrator = Orchestrator::recover(&mut reopened, timestamp(50)).unwrap();
    let _ = RoutineScheduler::recover(&mut reopened, timestamp(50)).unwrap();

    assert!(
        recovered_orchestrator
            .prepare_next(&mut reopened, timestamp(51))
            .unwrap()
            .is_none(),
        "recovery must not redeliver a routine prompt whose outcome is unknown"
    );
    assert_eq!(
        reopened
            .workspace(workspace_id)
            .unwrap()
            .routine_run(run_id)
            .unwrap()
            .step(RoutineStepId::new(1))
            .unwrap()
            .state(),
        RoutineStepState::Interrupted
    );
}

#[test]
fn cancelling_a_routine_task_from_the_timeline_does_not_start_a_retry() {
    let (_temp, mut manager, workspace_id) = manager();
    let retried = RoutineStep::new(
        RoutineStepId::new(1),
        name("Flaky"),
        AgentId::new(1),
        content("Try"),
        [],
        [],
        [],
        RoutineApproval::NotRequired,
        RoutineRetryPolicy::new(3).unwrap(),
        RoutineStepClaims::default(),
    )
    .unwrap();
    let routine_id = install(
        &mut manager,
        workspace_id,
        version(vec![retried], Vec::new()),
    );
    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(21))
        .unwrap();
    let task_id = manager
        .workspace(workspace_id)
        .unwrap()
        .routine_run(run_id)
        .unwrap()
        .step(RoutineStepId::new(1))
        .unwrap()
        .attempts()
        .last()
        .unwrap()
        .task_id();

    // The recovery control cancels the task directly, the way the timeline
    // panel does, without telling the scheduler.
    orchestrator
        .cancel_task(&mut manager, workspace_id, task_id, timestamp(30))
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(31))
        .unwrap();

    let run = manager
        .workspace(workspace_id)
        .unwrap()
        .routine_run(run_id)
        .unwrap();
    let step = run.step(RoutineStepId::new(1)).unwrap();
    assert_eq!(
        step.state(),
        RoutineStepState::Interrupted,
        "cancelling the conversation does not prove the agent stopped"
    );
    assert_eq!(step.attempts().len(), 1, "no retry ran alongside it");
    assert_eq!(
        run.held_reservations().len(),
        2,
        "the claims stay held until shutdown is confirmed"
    );
}

#[test]
fn an_undeclared_output_fails_the_step_instead_of_stalling_the_scheduler() {
    let (_temp, mut manager, workspace_id) = manager();
    let routine_id = install(
        &mut manager,
        workspace_id,
        version(
            vec![step(1, 1, "Investigate", vec![], vec![], vec!["finding"])],
            Vec::new(),
        ),
    );
    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(21))
        .unwrap();
    complete_through_orchestrator(
        &mut manager,
        &mut orchestrator,
        workspace_id,
        run_id,
        RoutineStepId::new(1),
        &[("finding", "found"), ("surprise", "unasked for")],
        30,
    );

    // The response is durable, so a rejected transition here would repeat on
    // every tick and stop the scheduler doing anything else.
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(40))
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(41))
        .unwrap();

    let run = manager
        .workspace(workspace_id)
        .unwrap()
        .routine_run(run_id)
        .unwrap();
    let step = run.step(RoutineStepId::new(1)).unwrap();
    assert_eq!(step.state(), RoutineStepState::Failed);
    assert!(
        step.failure().unwrap().as_str().contains("surprise"),
        "the reason names the undeclared output"
    );
    assert_eq!(run.state(), RoutineRunState::Failed);
}

#[test]
fn delivery_that_runs_out_of_retries_releases_the_step() {
    let (_temp, mut manager, workspace_id) = manager();
    let routine_id = simple_routine(&mut manager, workspace_id);
    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(21))
        .unwrap();

    // The recipient terminal never comes up, and the delivery window closes.
    let request = orchestrator
        .prepare_next(&mut manager, timestamp(22))
        .unwrap()
        .unwrap();
    orchestrator
        .finish_delivery(
            &mut manager,
            &request,
            Err("recipient terminal is not running".to_owned()),
            timestamp(40_000),
        )
        .unwrap();

    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(40_001))
        .unwrap();
    let run = manager
        .workspace(workspace_id)
        .unwrap()
        .routine_run(run_id)
        .unwrap();
    let step = run.step(RoutineStepId::new(1)).unwrap();
    assert_eq!(
        step.state(),
        RoutineStepState::Failed,
        "a prompt that can never be delivered must not hold the step forever"
    );
    assert!(
        run.held_reservations().is_empty(),
        "its claims are released"
    );
}

#[test]
fn a_step_whose_agent_left_its_claimed_checkout_does_not_run() {
    let (temp, mut manager, workspace_id) = manager();
    let routine_id = simple_routine(&mut manager, workspace_id);
    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    let run_id = scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();

    // Move the bound agent onto a different checkout after the run pinned it.
    let elsewhere = temp.path().join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    let mut floors = manager.workspace(workspace_id).unwrap().floors().clone();
    floors.entries.insert(
        1,
        crate::domain::Floor {
            name: name("other"),
            directory: crate::domain::WorkspaceDirectory::new(
                elsewhere.to_str().unwrap().to_owned(),
            )
            .unwrap(),
            repository: crate::domain::WorkspaceDirectory::new(
                elsewhere.to_str().unwrap().to_owned(),
            )
            .unwrap(),
            branch: None,
            base_revision: "abc".to_owned(),
            base_branch: None,
            managed: false,
            ownership_token: None,
            owner: None,
            dirty: false,
            lifecycle: crate::domain::FloorLifecycle::Available,
        },
    );
    let before = manager.workspace(workspace_id).unwrap().floors().clone();
    manager
        .execute(
            workspace_id,
            DomainCommand::ReplaceFloors {
                before,
                after: floors.clone(),
            },
            timestamp(21),
        )
        .unwrap();
    // Put the agent's node on that floor.
    manager
        .execute(
            workspace_id,
            DomainCommand::AddNode(crate::domain::Node::new(
                crate::domain::NodeId::new(9),
                crate::domain::NodeTarget::Agent(AgentId::new(1)),
                crate::domain::CanvasPoint::new(0.0, 0.0).unwrap(),
                crate::domain::CanvasSize::new(320.0, 200.0).unwrap(),
            )),
            timestamp(22),
        )
        .unwrap();
    let before = manager.workspace(workspace_id).unwrap().floors().clone();
    let mut moved = before.clone();
    moved.node_floors.insert(crate::domain::NodeId::new(9), 1);
    manager
        .execute(
            workspace_id,
            DomainCommand::ReplaceFloors {
                before,
                after: moved,
            },
            timestamp(23),
        )
        .unwrap();

    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(24))
        .unwrap();

    let run = manager
        .workspace(workspace_id)
        .unwrap()
        .routine_run(run_id)
        .unwrap();
    let step = run.step(RoutineStepId::new(1)).unwrap();
    assert_eq!(
        step.state(),
        RoutineStepState::Cancelled,
        "a step must not run in a checkout it never reserved"
    );
    assert!(step.attempts().is_empty(), "nothing was dispatched");
    assert!(
        step.failure().unwrap().as_str().contains("runs in"),
        "the reason names the checkout the agent actually uses"
    );
}

#[test]
fn two_workspaces_sharing_a_checkout_do_not_both_dispatch() {
    let temp = TempDir::new().unwrap();
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    let mut manager = WorkspaceManager::open(temp.path().join("openpodium.sqlite")).unwrap();

    // Both workspaces open the same directory, so their checkouts are the same
    // place on disk even though every identifier differs.
    let mut ids = Vec::new();
    for index in 0..2 {
        let workspace_id = manager
            .create_workspace(&project, timestamp(1 + index))
            .unwrap();
        manager
            .execute(
                workspace_id,
                DomainCommand::AddAgent(Agent::with_program(
                    AgentId::new(1),
                    name("Builder"),
                    None,
                    AgentProgram::Codex,
                )),
                timestamp(3 + index),
            )
            .unwrap();
        install(
            &mut manager,
            workspace_id,
            version(
                vec![step(1, 1, "Investigate", vec![], vec![], vec![])],
                Vec::new(),
            ),
        );
        ids.push(workspace_id);
    }

    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    for workspace_id in &ids {
        scheduler
            .start_run(
                &mut manager,
                *workspace_id,
                RoutineId::new(1),
                RunRequest::default(),
                timestamp(20),
            )
            .unwrap();
    }
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(21))
        .unwrap();

    let dispatched = ids
        .iter()
        .filter(|workspace_id| {
            manager
                .workspace(**workspace_id)
                .unwrap()
                .routine_runs()
                .any(|run| {
                    run.step(RoutineStepId::new(1)).unwrap().state() == RoutineStepState::Dispatched
                })
        })
        .count();
    assert_eq!(
        dispatched, 1,
        "checkout exclusivity has to hold across workspaces, because a checkout is a path"
    );
}

#[test]
fn a_workspace_with_a_routine_run_still_exports() {
    let (_temp, mut manager, workspace_id) = manager();
    let routine_id = simple_routine(&mut manager, workspace_id);
    let mut scheduler = RoutineScheduler::default();
    let mut orchestrator = Orchestrator::default();
    scheduler
        .start_run(
            &mut manager,
            workspace_id,
            routine_id,
            RunRequest::default(),
            timestamp(20),
        )
        .unwrap();
    scheduler
        .tick(&mut manager, &mut orchestrator, timestamp(21))
        .unwrap();
    let routine_handoff = manager
        .workspace(workspace_id)
        .unwrap()
        .handoffs()
        .find(|handoff| handoff.source().is_none())
        .map(|handoff| handoff.id())
        .expect("the run dispatched a routine-submitted handoff");
    assert!(
        manager
            .workspace(workspace_id)
            .unwrap()
            .handoff(routine_handoff)
            .is_some(),
        "the run dispatched a routine-submitted handoff"
    );

    let archive = manager
        .export_workspace_archive(workspace_id)
        .expect("a running routine must not break workspace export");
    assert!(
        !archive.contains("routine-1-1-1"),
        "the execution record of a run is not reusable workspace structure"
    );

    for (node_id, target, x) in [
        (
            crate::domain::NodeId::new(1),
            crate::domain::NodeTarget::Agent(AgentId::new(1)),
            0.0,
        ),
        (
            crate::domain::NodeId::new(2),
            crate::domain::NodeTarget::Handoff(routine_handoff),
            400.0,
        ),
    ] {
        manager
            .execute(
                workspace_id,
                DomainCommand::AddNode(crate::domain::Node::new(
                    node_id,
                    target,
                    crate::domain::CanvasPoint::new(x, 0.0).unwrap(),
                    crate::domain::CanvasSize::new(320.0, 200.0).unwrap(),
                )),
                timestamp(22),
            )
            .unwrap();
    }
    let template = manager
        .export_template(
            workspace_id,
            &[crate::domain::NodeId::new(1), crate::domain::NodeId::new(2)],
        )
        .expect("a reusable selection should ignore its routine execution record");
    assert!(!template.contains("routine-1-1-1"));
}
