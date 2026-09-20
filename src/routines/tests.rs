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
fn resolving_an_interruption_retries_within_the_budget() {
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
    let run = manager
        .workspace(workspace_id)
        .unwrap()
        .routine_run(run_id)
        .unwrap();
    let step = run.step(RoutineStepId::new(1)).unwrap();
    assert_eq!(step.attempts().len(), 1);
    assert_eq!(
        step.attempts()[0].task_id(),
        first_task,
        "a retry keeps the previous attempt and its task"
    );
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
