//! Read models for the durable orchestration timeline and recovery controls.

use std::collections::BTreeSet;

use crate::domain::{
    AgentId, AgentState, DeliveryOutcome, DomainEvent, Handoff, HandoffPayload,
    HandoffResponseStatus, HandoffTermination, NodeId, NodeTarget, Task, TaskId, TaskState,
    TimelineEvent, TimelineEventId, Timestamp, Workspace, WorkspaceId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttentionLevel {
    None,
    Informational,
    Urgent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    Inspect,
    Retry,
    Cancel,
    Resume,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AttentionCounts {
    pub blocked: usize,
    pub failed: usize,
}

impl AttentionCounts {
    pub const fn total(self) -> usize {
        self.blocked + self.failed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSummary {
    id: TaskId,
    title: String,
    state: TaskState,
    assignee: Option<AgentId>,
    retry_of: Option<TaskId>,
    needs_attention: bool,
}

impl TaskSummary {
    pub const fn id(&self) -> TaskId {
        self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub const fn state(&self) -> TaskState {
        self.state
    }

    pub const fn assignee(&self) -> Option<AgentId> {
        self.assignee
    }

    pub const fn retry_of(&self) -> Option<TaskId> {
        self.retry_of
    }

    pub const fn needs_attention(&self) -> bool {
        self.needs_attention
    }

    pub fn actions(&self) -> Vec<RecoveryAction> {
        recovery_actions(self.state)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineItem {
    event_id: TimelineEventId,
    occurred_at: Timestamp,
    agent_id: Option<AgentId>,
    task_id: Option<TaskId>,
    title: String,
    detail: String,
    attention: AttentionLevel,
}

impl TimelineItem {
    pub const fn event_id(&self) -> TimelineEventId {
        self.event_id
    }

    pub const fn occurred_at(&self) -> Timestamp {
        self.occurred_at
    }

    pub const fn agent_id(&self) -> Option<AgentId> {
        self.agent_id
    }

    pub const fn task_id(&self) -> Option<TaskId> {
        self.task_id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }

    pub const fn attention(&self) -> AttentionLevel {
        self.attention
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavigationTarget {
    pub workspace_id: WorkspaceId,
    pub task_id: TaskId,
    pub node_id: Option<NodeId>,
}

pub fn project(workspace: &Workspace, events: &[TimelineEvent]) -> Vec<TimelineItem> {
    events
        .iter()
        .map(|event| project_event(workspace, event))
        .collect()
}

pub fn task_summaries(workspace: &Workspace) -> Vec<TaskSummary> {
    let retried = retried_tasks(workspace);
    workspace
        .tasks()
        .map(|task| TaskSummary {
            id: task.id(),
            title: task.title().as_str().to_owned(),
            state: task.state(),
            assignee: task.assignee(),
            retry_of: task.retry_of(),
            needs_attention: task.state() == TaskState::Blocked
                || (task.state() == TaskState::Failed && !retried.contains(&task.id())),
        })
        .collect()
}

pub fn attention_counts(workspace: &Workspace) -> AttentionCounts {
    let retried = retried_tasks(workspace);
    workspace
        .tasks()
        .fold(AttentionCounts::default(), |mut counts, task| {
            match task.state() {
                TaskState::Blocked => counts.blocked += 1,
                TaskState::Failed if !retried.contains(&task.id()) => counts.failed += 1,
                TaskState::Failed => {}
                TaskState::Queued
                | TaskState::Delivered
                | TaskState::Running
                | TaskState::Completed
                | TaskState::Cancelled => {}
            }
            counts
        })
}

fn retried_tasks(workspace: &Workspace) -> BTreeSet<TaskId> {
    workspace.tasks().filter_map(Task::retry_of).collect()
}

pub fn navigation_target(workspace: &Workspace, task_id: TaskId) -> Option<NavigationTarget> {
    let task = workspace.task(task_id)?;
    let task_node = workspace
        .nodes()
        .find(|node| node.reference() == Some(NodeTarget::Task(task_id)))
        .map(|node| node.id());
    let assignee_node = task.assignee().and_then(|assignee| {
        workspace
            .nodes()
            .find(|node| node.reference() == Some(NodeTarget::Agent(assignee)))
            .map(|node| node.id())
    });
    Some(NavigationTarget {
        workspace_id: workspace.id(),
        task_id,
        node_id: task_node.or(assignee_node),
    })
}

pub fn recovery_actions(state: TaskState) -> Vec<RecoveryAction> {
    let mut actions = vec![RecoveryAction::Inspect];
    match state {
        TaskState::Queued | TaskState::Delivered | TaskState::Running => {
            actions.push(RecoveryAction::Cancel);
        }
        TaskState::Blocked => {
            actions.push(RecoveryAction::Resume);
            actions.push(RecoveryAction::Cancel);
        }
        TaskState::Failed | TaskState::Cancelled => actions.push(RecoveryAction::Retry),
        TaskState::Completed => {}
    }
    actions
}

fn project_event(workspace: &Workspace, event: &TimelineEvent) -> TimelineItem {
    let (task_id, title, detail) = match event.event() {
        DomainEvent::FloorsChanged { .. } => (
            None,
            "Worktree floors updated".to_owned(),
            "Floor inventory or selection changed".to_owned(),
        ),
        DomainEvent::WorkspaceSettingsChanged { to, .. } => (
            None,
            "Workspace settings updated".to_owned(),
            to.name().as_str().to_owned(),
        ),
        DomainEvent::EnvironmentProfileAdded(profile) => (
            None,
            "Environment added".to_owned(),
            profile.name().as_str().to_owned(),
        ),
        DomainEvent::EnvironmentProfileChanged { to, .. } => (
            None,
            "Environment updated".to_owned(),
            to.name().as_str().to_owned(),
        ),
        DomainEvent::EnvironmentProfileRemoved(profile) => (
            None,
            "Environment removed".to_owned(),
            profile.name().as_str().to_owned(),
        ),
        DomainEvent::CommandPresetAdded(preset) => (
            None,
            "Command preset added".to_owned(),
            preset.name().as_str().to_owned(),
        ),
        DomainEvent::CommandPresetChanged { to, .. } => (
            None,
            "Command preset updated".to_owned(),
            to.name().as_str().to_owned(),
        ),
        DomainEvent::CommandPresetRemoved(preset) => (
            None,
            "Command preset removed".to_owned(),
            preset.name().as_str().to_owned(),
        ),
        DomainEvent::RoleAdded(role) => (
            None,
            "Role added".to_owned(),
            role.name().as_str().to_owned(),
        ),
        DomainEvent::RoleChanged { to, .. } => (
            None,
            "Role updated".to_owned(),
            to.name().as_str().to_owned(),
        ),
        DomainEvent::RoleRemoved(role) => (
            None,
            "Role removed".to_owned(),
            role.name().as_str().to_owned(),
        ),
        DomainEvent::AgentRoleChanged { agent_id, .. } => (
            None,
            "Agent role updated".to_owned(),
            agent_label(workspace, *agent_id),
        ),
        DomainEvent::AgentAdded(agent) => (
            None,
            "Agent added".to_owned(),
            format!("{} is {}", agent.name(), agent.state()),
        ),
        DomainEvent::AgentRenamed { from, to, .. } => {
            (None, "Agent renamed".to_owned(), format!("{from} → {to}"))
        }
        DomainEvent::ChatThreadAdded(thread) => (
            None,
            "Chat thread added".to_owned(),
            thread.name().as_str().to_owned(),
        ),
        DomainEvent::ChatThreadChanged { to_name, .. } => (
            None,
            "Chat thread updated".to_owned(),
            to_name.as_str().to_owned(),
        ),
        DomainEvent::ChatAttachmentAdded(attachment) => (
            None,
            "Chat attachment added".to_owned(),
            attachment.display_name().to_owned(),
        ),
        DomainEvent::ChatDraftChanged { thread_id, .. } => (
            None,
            "Chat draft saved".to_owned(),
            format!("Thread {thread_id}"),
        ),
        DomainEvent::ChatDraftSubmitted { message, .. }
        | DomainEvent::AgentChatMessageAppended(message) => (
            None,
            "Chat message recorded".to_owned(),
            format!("Message {}", message.id()),
        ),
        DomainEvent::TaskAdded(task) => task_added(workspace, task),
        DomainEvent::HandoffAdded(handoff) => handoff_added(workspace, handoff),
        DomainEvent::TaskHandoffAdded { task, .. } => task_added(workspace, task),
        DomainEvent::HandoffChanged { before, after } => handoff_changed(before, after),
        DomainEvent::TaskCancelled { task_id, from, .. } => (
            Some(*task_id),
            "Task cancelled".to_owned(),
            format!(
                "{}: {from} → {}",
                task_label(workspace, *task_id),
                TaskState::Cancelled
            ),
        ),
        DomainEvent::TaskResumed { task_id, from, .. } => (
            Some(*task_id),
            "Task resumed".to_owned(),
            format!(
                "{}: {from} → {}",
                task_label(workspace, *task_id),
                TaskState::Running
            ),
        ),
        DomainEvent::NodeAdded(node) => (
            node.reference().and_then(task_from_target),
            "Canvas node added".to_owned(),
            format!("Node {}", node.id()),
        ),
        DomainEvent::AgentNodeAdded { agent, node } => (
            None,
            "Agent added to canvas".to_owned(),
            format!("{} on node {}", agent.name(), node.id()),
        ),
        DomainEvent::CanvasReplaced { after, .. } => (
            None,
            "Canvas updated".to_owned(),
            format!("{} nodes", after.nodes().len()),
        ),
        DomainEvent::AgentStateChanged { agent_id, from, to } => (
            None,
            "Agent state changed".to_owned(),
            format!("{}: {from} → {to}", agent_label(workspace, *agent_id)),
        ),
        DomainEvent::TaskStateChanged { task_id, from, to } => (
            Some(*task_id),
            "Task state changed".to_owned(),
            format!("{}: {from} → {to}", task_label(workspace, *task_id)),
        ),
        DomainEvent::RoutineAdded(routine) => (
            None,
            "Routine added".to_owned(),
            routine.name().as_str().to_owned(),
        ),
        DomainEvent::RoutineVersionAdded {
            routine_id,
            version,
        } => (
            None,
            "Routine version saved".to_owned(),
            format!(
                "{} version {}",
                routine_label(workspace, *routine_id),
                version.number()
            ),
        ),
        DomainEvent::RoutineChanged { to_name, .. } => (
            None,
            "Routine updated".to_owned(),
            to_name.as_str().to_owned(),
        ),
        DomainEvent::RoutineTriggerChanged { to, .. } => (
            None,
            if to.enabled() {
                "Routine trigger enabled".to_owned()
            } else {
                "Routine trigger disabled".to_owned()
            },
            format!("{} ({})", to.name(), to.kind().label()),
        ),
        DomainEvent::RoutineTriggerRemoved { trigger, .. } => (
            None,
            "Routine trigger removed".to_owned(),
            trigger.name().as_str().to_owned(),
        ),
        DomainEvent::RoutineRunStarted { run, .. } => (
            None,
            "Routine run started".to_owned(),
            format!(
                "{} version {} as run {}",
                routine_label(workspace, run.routine_id()),
                run.pin().version().number(),
                run.id()
            ),
        ),
        DomainEvent::RoutineOccurrencesSkipped {
            trigger_id,
            skipped,
            ..
        } => (
            None,
            "Scheduled runs skipped".to_owned(),
            format!(
                "Trigger {trigger_id} missed {skipped} occurrence(s) while OpenPodium was closed"
            ),
        ),
        DomainEvent::RoutineRunAdvanced { run_id, transition } => {
            routine_transition(workspace, *run_id, transition)
        }
        DomainEvent::RoutineStepDispatched {
            run_id,
            step_id,
            task,
            ..
        } => (
            Some(task.id()),
            "Routine step dispatched".to_owned(),
            format!("Run {run_id} step {step_id}: {}", task.title()),
        ),
    };
    let agent_id = match event.event() {
        DomainEvent::AgentStateChanged { agent_id, .. } => Some(*agent_id),
        _ => None,
    };
    let attention = match event.event() {
        DomainEvent::AgentStateChanged {
            to: AgentState::Failed,
            ..
        } => AttentionLevel::Urgent,
        DomainEvent::AgentStateChanged {
            to: AgentState::Completed,
            ..
        } => AttentionLevel::Informational,
        DomainEvent::TaskStateChanged {
            to: TaskState::Blocked | TaskState::Failed,
            ..
        } => AttentionLevel::Urgent,
        DomainEvent::TaskStateChanged {
            to: TaskState::Completed,
            ..
        } => AttentionLevel::Informational,
        // A step that needs a decision, failed, or was interrupted is waiting
        // on the user; nothing moves it forward on its own.
        DomainEvent::RoutineRunAdvanced { transition, .. } => match transition {
            crate::domain::RoutineTransition::AwaitApproval { .. }
            | crate::domain::RoutineTransition::FailStep { .. }
            | crate::domain::RoutineTransition::InterruptStep { .. } => AttentionLevel::Urgent,
            crate::domain::RoutineTransition::CompleteStep { .. }
            | crate::domain::RoutineTransition::Settle { .. } => AttentionLevel::Informational,
            _ => AttentionLevel::None,
        },
        DomainEvent::RoutineOccurrencesSkipped { .. } => AttentionLevel::Urgent,
        _ => AttentionLevel::None,
    };
    TimelineItem {
        event_id: event.id(),
        occurred_at: event.occurred_at(),
        agent_id,
        task_id,
        title,
        detail,
        attention,
    }
}

fn task_added(workspace: &Workspace, task: &Task) -> (Option<TaskId>, String, String) {
    let retry = task
        .retry_of()
        .map_or_else(String::new, |task_id| format!("; retry of task {task_id}"));
    (
        Some(task.id()),
        "Task queued".to_owned(),
        format!(
            "{} assigned to {}{retry}",
            task.title(),
            task.assignee().map_or_else(
                || "nobody".to_owned(),
                |agent_id| agent_label(workspace, agent_id)
            )
        ),
    )
}

fn routine_label(workspace: &Workspace, routine_id: crate::domain::RoutineId) -> String {
    workspace.routine(routine_id).map_or_else(
        || format!("Routine {routine_id}"),
        |routine| routine.name().as_str().to_owned(),
    )
}

/// Renders a run transition, attributing it to the task the step dispatched so
/// the timeline filter keeps routine work grouped with its task history.
fn routine_transition(
    workspace: &Workspace,
    run_id: crate::domain::RoutineRunId,
    transition: &crate::domain::RoutineTransition,
) -> (Option<TaskId>, String, String) {
    use crate::domain::RoutineTransition as Transition;
    let step_id = match transition {
        Transition::AwaitApproval { step_id }
        | Transition::RecordApproval { step_id, .. }
        | Transition::CompleteStep { step_id, .. }
        | Transition::FailStep { step_id, .. }
        | Transition::InterruptStep { step_id, .. }
        | Transition::ResolveInterruption { step_id, .. }
        | Transition::CancelStep { step_id, .. } => Some(*step_id),
        Transition::RequestCancellation { .. } | Transition::Settle { .. } => None,
    };
    let task_id = step_id.and_then(|step_id| {
        workspace
            .routine_run(run_id)?
            .step(step_id)?
            .attempts()
            .last()
            .map(crate::domain::RoutineAttempt::task_id)
    });
    let (title, detail) = match transition {
        Transition::AwaitApproval { step_id } => (
            "Routine step awaiting approval",
            format!("Run {run_id} step {step_id}"),
        ),
        Transition::RecordApproval { step_id, record } => (
            match record.decision() {
                crate::domain::RoutineApprovalDecision::Approved => "Routine step approved",
                crate::domain::RoutineApprovalDecision::Rejected => "Routine step rejected",
            },
            format!("Run {run_id} step {step_id}"),
        ),
        Transition::CompleteStep {
            step_id, outputs, ..
        } => (
            "Routine step completed",
            format!("Run {run_id} step {step_id}: {} output(s)", outputs.len()),
        ),
        Transition::FailStep {
            step_id, reason, ..
        } => (
            "Routine step failed",
            format!(
                "Run {run_id} step {step_id}: {reason}",
                reason = reason.as_str()
            ),
        ),
        Transition::InterruptStep {
            step_id, reason, ..
        } => (
            "Routine step interrupted",
            format!("Run {run_id} step {step_id}: {reason}"),
        ),
        Transition::ResolveInterruption { step_id, .. } => (
            "Routine interruption resolved",
            format!("Run {run_id} step {step_id}"),
        ),
        Transition::CancelStep { step_id, .. } => (
            "Routine step cancelled",
            format!("Run {run_id} step {step_id}"),
        ),
        Transition::RequestCancellation { .. } => (
            "Routine run cancelling",
            format!("Run {run_id} stopped dispatching further steps"),
        ),
        Transition::Settle { .. } => (
            "Routine run finished",
            workspace.routine_run(run_id).map_or_else(
                || format!("Run {run_id}"),
                |run| format!("Run {run_id} is {}", run.state()),
            ),
        ),
    };
    (task_id, title.to_owned(), detail)
}

fn origin_label(workspace: &Workspace, handoff: &Handoff) -> String {
    match handoff.origin() {
        crate::domain::HandoffOrigin::Agent(agent_id) => agent_label(workspace, agent_id),
        crate::domain::HandoffOrigin::Routine { run_id, step_id } => {
            format!("Routine run {run_id} step {step_id}")
        }
    }
}

fn handoff_added(workspace: &Workspace, handoff: &Handoff) -> (Option<TaskId>, String, String) {
    let task_id = handoff_task_id(handoff);
    (
        task_id,
        "Handoff queued".to_owned(),
        format!(
            "{} → {}",
            origin_label(workspace, handoff),
            agent_label(workspace, handoff.recipient())
        ),
    )
}

fn handoff_changed(before: &Handoff, after: &Handoff) -> (Option<TaskId>, String, String) {
    let task_id = handoff_task_id(after);
    if after.delivery_attempts().len() > before.delivery_attempts().len() {
        let attempt = after
            .delivery_attempts()
            .last()
            .expect("a delivery attempt was appended");
        return (
            task_id,
            "Delivery started".to_owned(),
            format!("Attempt {} via {}", attempt.ordinal(), attempt.mechanism()),
        );
    }
    if let Some(attempt) = after
        .delivery_attempts()
        .iter()
        .zip(before.delivery_attempts())
        .find_map(|(after, before)| (after != before).then_some(after))
    {
        let (title, detail) = match attempt.outcome() {
            DeliveryOutcome::Started => (
                "Delivery started".to_owned(),
                format!("Attempt {}", attempt.ordinal()),
            ),
            DeliveryOutcome::Delivered { .. } => (
                "Delivery completed".to_owned(),
                format!("Attempt {} reached the agent", attempt.ordinal()),
            ),
            DeliveryOutcome::Failed {
                error, retryable, ..
            } => (
                "Delivery failed".to_owned(),
                format!(
                    "Attempt {}: {}{}",
                    attempt.ordinal(),
                    error.as_str(),
                    if *retryable { "; retry scheduled" } else { "" }
                ),
            ),
        };
        return (task_id, title, detail);
    }
    if after.progress().len() > before.progress().len() {
        let progress = after.progress().last().expect("progress was appended");
        return (
            task_id,
            "Progress reported".to_owned(),
            progress.body().as_str().to_owned(),
        );
    }
    if after.response() != before.response()
        && let Some(response) = after.response()
    {
        let status = match response.status() {
            HandoffResponseStatus::Completed => "completed",
            HandoffResponseStatus::Failed => "failed",
            HandoffResponseStatus::Blocked => "blocked",
        };
        return (
            task_id,
            format!("Agent reported {status}"),
            response.body().as_str().to_owned(),
        );
    }
    if after.termination() != before.termination()
        && let Some(termination) = after.termination()
    {
        return match termination {
            HandoffTermination::Cancelled { reason, .. } => (
                task_id,
                "Handoff cancelled".to_owned(),
                reason.as_str().to_owned(),
            ),
            HandoffTermination::TimedOut { .. } => (
                task_id,
                "Handoff timed out".to_owned(),
                "The response deadline expired".to_owned(),
            ),
        };
    }
    (
        task_id,
        "Handoff updated".to_owned(),
        format!("Handoff {}", after.id()),
    )
}

fn handoff_task_id(handoff: &Handoff) -> Option<TaskId> {
    match handoff.payload() {
        HandoffPayload::Task(task_id) => Some(*task_id),
        HandoffPayload::Question(_) => None,
    }
}

fn task_from_target(target: NodeTarget) -> Option<TaskId> {
    match target {
        NodeTarget::Task(task_id) => Some(task_id),
        NodeTarget::Agent(_) | NodeTarget::Handoff(_) => None,
    }
}

fn task_label(workspace: &Workspace, task_id: TaskId) -> String {
    workspace.task(task_id).map_or_else(
        || format!("Task {task_id}"),
        |task| task.title().as_str().to_owned(),
    )
}

fn agent_label(workspace: &Workspace, agent_id: AgentId) -> String {
    workspace.agent(agent_id).map_or_else(
        || format!("Agent {agent_id}"),
        |agent| agent.name().as_str().to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        Agent, CanvasPoint, CanvasSize, Content, DeliveryMechanism, DomainCommand, HandoffId,
        HandoffMessageId, HandoffOrigin, Name, Node, TimelineEventId,
    };

    #[test]
    fn task_projection_is_ordered_scoped_and_actionable() {
        let workspace_id = WorkspaceId::new(1);
        let mut workspace = Workspace::new(workspace_id, Name::new("Test").unwrap());
        workspace
            .execute(DomainCommand::AddAgent(Agent::new(
                AgentId::new(1),
                Name::new("Lead").unwrap(),
                None,
            )))
            .unwrap();
        workspace
            .execute(DomainCommand::AddAgent(Agent::new(
                AgentId::new(2),
                Name::new("Builder").unwrap(),
                None,
            )))
            .unwrap();
        workspace
            .execute(DomainCommand::AddNode(Node::new(
                NodeId::new(7),
                NodeTarget::Agent(AgentId::new(2)),
                CanvasPoint::new(0.0, 0.0).unwrap(),
                CanvasSize::new(100.0, 100.0).unwrap(),
            )))
            .unwrap();
        let task_id = TaskId::new(1);
        let task = Task::new(
            task_id,
            Name::new("Fix timeline").unwrap(),
            Content::new("Implement it").unwrap(),
            Some(AgentId::new(2)),
            None,
        );
        let handoff = Handoff::tracked(
            HandoffId::new(1),
            HandoffMessageId::new("task-1").unwrap(),
            HandoffOrigin::Agent(AgentId::new(1)),
            AgentId::new(2),
            HandoffPayload::Task(task_id),
            None,
            Timestamp::from_unix_millis(10),
            None,
        )
        .unwrap();
        let queued = workspace
            .execute(DomainCommand::AddTaskHandoff { task, handoff })
            .unwrap();
        let delivered = workspace
            .execute(DomainCommand::TransitionTask {
                task_id,
                to: TaskState::Delivered,
            })
            .unwrap();
        let running = workspace
            .execute(DomainCommand::TransitionTask {
                task_id,
                to: TaskState::Running,
            })
            .unwrap();
        let blocked = workspace
            .execute(DomainCommand::TransitionTask {
                task_id,
                to: TaskState::Blocked,
            })
            .unwrap();
        let events = [queued, delivered, running, blocked]
            .into_iter()
            .enumerate()
            .map(|(index, event)| {
                TimelineEvent::new(
                    TimelineEventId::new(index as u64 + 1),
                    workspace_id,
                    Timestamp::from_unix_millis(index as u64 + 10),
                    event,
                )
            })
            .collect::<Vec<_>>();

        let items = project(&workspace, &events);

        assert_eq!(items.len(), 4);
        assert!(items.iter().all(|item| item.task_id() == Some(task_id)));
        assert_eq!(items.last().unwrap().attention(), AttentionLevel::Urgent);
        assert_eq!(attention_counts(&workspace).blocked, 1);
        assert_eq!(
            task_summaries(&workspace)[0].actions(),
            vec![
                RecoveryAction::Inspect,
                RecoveryAction::Resume,
                RecoveryAction::Cancel,
            ]
        );
        assert_eq!(
            navigation_target(&workspace, task_id).unwrap().node_id,
            Some(NodeId::new(7))
        );
    }

    #[test]
    fn delivery_attempt_changes_have_task_scope() {
        let workspace_id = WorkspaceId::new(1);
        let mut workspace = Workspace::new(workspace_id, Name::new("Test").unwrap());
        let task_id = TaskId::new(1);
        let task = Task::new(
            task_id,
            Name::new("Deliver").unwrap(),
            Content::new("Prompt").unwrap(),
            None,
            None,
        );
        workspace.execute(DomainCommand::AddTask(task)).unwrap();
        let before = Handoff::tracked(
            HandoffId::new(1),
            HandoffMessageId::new("task-1").unwrap(),
            HandoffOrigin::Agent(AgentId::new(1)),
            AgentId::new(2),
            HandoffPayload::Task(task_id),
            None,
            Timestamp::from_unix_millis(10),
            None,
        )
        .unwrap();
        let mut after = before.clone();
        after
            .begin_delivery(
                HandoffMessageId::new("task-1").unwrap(),
                DeliveryMechanism::CodexTerminal,
                Timestamp::from_unix_millis(11),
            )
            .unwrap();
        let event = TimelineEvent::new(
            TimelineEventId::new(1),
            workspace_id,
            Timestamp::from_unix_millis(11),
            DomainEvent::HandoffChanged { before, after },
        );

        let item = project(&workspace, &[event]).remove(0);

        assert_eq!(item.task_id(), Some(task_id));
        assert_eq!(item.title(), "Delivery started");
    }
}
