//! Local, evidence-linked supervision across loaded workspaces.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::domain::{AgentState, TaskState, TimelineEventId, Timestamp, Workspace, WorkspaceId};
use crate::timeline::{AttentionLevel, TimelineItem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SignalClass {
    Attention,
    Completion,
    Failure,
    Collision,
}

impl SignalClass {
    pub const ALL: [Self; 4] = [
        Self::Attention,
        Self::Completion,
        Self::Failure,
        Self::Collision,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Attention => "attention",
            Self::Completion => "completion",
            Self::Failure => "failure",
            Self::Collision => "collision",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evidence {
    Timeline {
        workspace_id: WorkspaceId,
        event_id: TimelineEventId,
    },
    Collision {
        workspace_id: WorkspaceId,
        path: String,
        left_checkout: String,
        right_checkout: String,
    },
}

impl Evidence {
    pub const fn workspace_id(&self) -> WorkspaceId {
        match self {
            Self::Timeline { workspace_id, .. } | Self::Collision { workspace_id, .. } => {
                *workspace_id
            }
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Timeline {
                workspace_id,
                event_id,
            } => format!("workspace {workspace_id}, event {event_id}"),
            Self::Collision {
                path,
                left_checkout,
                right_checkout,
                ..
            } => format!("{path}: {left_checkout} ↔ {right_checkout}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    workspace_id: WorkspaceId,
    class: SignalClass,
    text: String,
    evidence: Vec<Evidence>,
}

impl Claim {
    fn new(
        workspace_id: WorkspaceId,
        class: SignalClass,
        text: String,
        evidence: Vec<Evidence>,
    ) -> Option<Self> {
        (!evidence.is_empty()).then_some(Self {
            workspace_id,
            class,
            text,
            evidence,
        })
    }

    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub const fn class(&self) -> SignalClass {
        self.class
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn evidence(&self) -> &[Evidence] {
        &self.evidence
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recommendation {
    workspace_id: WorkspaceId,
    text: String,
    evidence: Vec<Evidence>,
}

impl Recommendation {
    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn evidence(&self) -> &[Evidence] {
        &self.evidence
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollisionObservation {
    pub workspace_id: WorkspaceId,
    pub path: String,
    pub left_checkout: String,
    pub right_checkout: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    claims: Vec<Claim>,
    recommendations: Vec<Recommendation>,
}

impl Snapshot {
    pub fn claims(&self) -> &[Claim] {
        &self.claims
    }

    pub fn recommendations(&self) -> &[Recommendation] {
        &self.recommendations
    }

    pub fn count(&self, class: SignalClass) -> usize {
        self.claims
            .iter()
            .filter(|claim| claim.class == class)
            .map(|claim| claim.evidence.len())
            .sum()
    }
}

pub struct WorkspaceActivity<'a> {
    pub workspace: &'a Workspace,
    pub timeline: &'a [TimelineItem],
}

pub fn aggregate<'a>(
    workspaces: impl IntoIterator<Item = WorkspaceActivity<'a>>,
    collisions: impl IntoIterator<Item = CollisionObservation>,
) -> Snapshot {
    let mut snapshot = Snapshot::default();
    for activity in workspaces {
        add_workspace_claims(&mut snapshot, activity);
    }

    let mut by_workspace = BTreeMap::<WorkspaceId, Vec<Evidence>>::new();
    for collision in collisions {
        by_workspace
            .entry(collision.workspace_id)
            .or_default()
            .push(Evidence::Collision {
                workspace_id: collision.workspace_id,
                path: collision.path,
                left_checkout: collision.left_checkout,
                right_checkout: collision.right_checkout,
            });
    }
    for (workspace_id, evidence) in by_workspace {
        let count = evidence.len();
        push_claim(
            &mut snapshot,
            workspace_id,
            SignalClass::Collision,
            format!("{count} overlapping path(s) need review"),
            evidence,
            "Review the collision preview before choosing an integration action.",
        );
    }
    snapshot
}

fn add_workspace_claims(snapshot: &mut Snapshot, activity: WorkspaceActivity<'_>) {
    let workspace_id = activity.workspace.id();
    let mut task_evidence = BTreeMap::<SignalClass, Vec<Evidence>>::new();
    for task in activity.workspace.tasks() {
        let class = match task.state() {
            TaskState::Blocked => Some(SignalClass::Attention),
            TaskState::Failed => Some(SignalClass::Failure),
            TaskState::Completed => Some(SignalClass::Completion),
            TaskState::Queued
            | TaskState::Delivered
            | TaskState::Running
            | TaskState::Cancelled => None,
        };
        let Some(class) = class else { continue };
        let item = activity.timeline.iter().rev().find(|item| {
            item.task_id() == Some(task.id())
                && match class {
                    SignalClass::Attention => item.attention() == AttentionLevel::Urgent,
                    SignalClass::Failure => {
                        item.attention() == AttentionLevel::Urgent
                            && (item.title().to_ascii_lowercase().contains("fail")
                                || item.detail().to_ascii_lowercase().contains("fail"))
                    }
                    SignalClass::Completion => item.attention() == AttentionLevel::Informational,
                    SignalClass::Collision => false,
                }
        });
        if let Some(item) = item {
            task_evidence
                .entry(class)
                .or_default()
                .push(Evidence::Timeline {
                    workspace_id,
                    event_id: item.event_id(),
                });
        }
    }

    for (class, evidence) in task_evidence {
        let count = evidence.len();
        let (claim, recommendation) = match class {
            SignalClass::Attention => (
                format!("{count} task(s) are blocked"),
                "Inspect blocked work and provide the missing decision or context.",
            ),
            SignalClass::Failure => (
                format!("{count} task(s) failed"),
                "Inspect the failure evidence before retrying the task.",
            ),
            SignalClass::Completion => (
                format!("{count} task(s) completed"),
                "Review completed work before integrating it.",
            ),
            SignalClass::Collision => unreachable!("collisions are aggregated separately"),
        };
        push_claim(
            snapshot,
            workspace_id,
            class,
            claim,
            evidence,
            recommendation,
        );
    }

    let mut agent_evidence = BTreeMap::<SignalClass, Vec<Evidence>>::new();
    for agent in activity.workspace.agents() {
        let class = match agent.state() {
            AgentState::Failed => Some(SignalClass::Failure),
            AgentState::Completed => Some(SignalClass::Completion),
            AgentState::Starting
            | AgentState::Running
            | AgentState::Waiting
            | AgentState::Stopped => None,
        };
        let Some(class) = class else { continue };
        let expected_end = format!("→ {}", agent.state());
        if let Some(item) = activity.timeline.iter().rev().find(|item| {
            item.agent_id() == Some(agent.id())
                && item.title() == "Agent state changed"
                && item.detail().ends_with(&expected_end)
        }) {
            agent_evidence
                .entry(class)
                .or_default()
                .push(Evidence::Timeline {
                    workspace_id,
                    event_id: item.event_id(),
                });
        }
    }
    for (class, evidence) in agent_evidence {
        let count = evidence.len();
        let (claim, recommendation) = match class {
            SignalClass::Failure => (
                format!("{count} agent(s) failed"),
                "Inspect the agent failure before restarting its work.",
            ),
            SignalClass::Completion => (
                format!("{count} agent(s) completed"),
                "Review the completed agent work before integrating it.",
            ),
            SignalClass::Attention | SignalClass::Collision => {
                unreachable!("agent states do not produce this class")
            }
        };
        push_claim(
            snapshot,
            workspace_id,
            class,
            claim,
            evidence,
            recommendation,
        );
    }
}

fn push_claim(
    snapshot: &mut Snapshot,
    workspace_id: WorkspaceId,
    class: SignalClass,
    text: String,
    evidence: Vec<Evidence>,
    recommendation: &str,
) {
    let Some(claim) = Claim::new(workspace_id, class, text, evidence) else {
        return;
    };
    snapshot.recommendations.push(Recommendation {
        workspace_id,
        text: recommendation.to_owned(),
        evidence: claim.evidence.clone(),
    });
    snapshot.claims.push(claim);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterResidency {
    OnDevice,
    External,
}

pub trait SummaryAdapter {
    fn residency(&self) -> AdapterResidency;
    fn summarize(&self, snapshot: &Snapshot) -> Result<String, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryOutput {
    pub text: String,
    pub used_adapter: bool,
    pub adapter_error: Option<String>,
}

pub fn summarize(
    snapshot: &Snapshot,
    adapter: Option<&dyn SummaryAdapter>,
    allow_external_project_content: bool,
) -> SummaryOutput {
    let fallback = rule_summary(snapshot);
    let Some(adapter) = adapter else {
        return SummaryOutput {
            text: fallback,
            used_adapter: false,
            adapter_error: None,
        };
    };
    if adapter.residency() == AdapterResidency::External && !allow_external_project_content {
        return SummaryOutput {
            text: fallback,
            used_adapter: false,
            adapter_error: Some(
                "external summary adapter requires explicit project-content permission".to_owned(),
            ),
        };
    }
    match adapter.summarize(snapshot) {
        Ok(text) if !text.trim().is_empty() => SummaryOutput {
            text,
            used_adapter: true,
            adapter_error: None,
        },
        Ok(_) => SummaryOutput {
            text: fallback,
            used_adapter: false,
            adapter_error: Some("summary adapter returned an empty summary".to_owned()),
        },
        Err(error) => SummaryOutput {
            text: fallback,
            used_adapter: false,
            adapter_error: Some(error),
        },
    }
}

fn rule_summary(snapshot: &Snapshot) -> String {
    if snapshot.claims.is_empty() {
        return "No attention, completion, failure, or collision signals.".to_owned();
    }
    snapshot
        .claims
        .iter()
        .map(|claim| claim.text.as_str())
        .collect::<Vec<_>>()
        .join(" · ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationRule {
    pub enabled: bool,
    pub cooldown: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationSettings {
    rules: BTreeMap<SignalClass, NotificationRule>,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            rules: SignalClass::ALL
                .into_iter()
                .map(|class| {
                    (
                        class,
                        NotificationRule {
                            enabled: true,
                            cooldown: Duration::from_secs(30),
                        },
                    )
                })
                .collect(),
        }
    }
}

impl NotificationSettings {
    pub fn rule(&self, class: SignalClass) -> NotificationRule {
        self.rules[&class]
    }

    pub fn toggle(&mut self, class: SignalClass) {
        if let Some(rule) = self.rules.get_mut(&class) {
            rule.enabled = !rule.enabled;
        }
    }

    pub fn cycle_cooldown(&mut self, class: SignalClass) {
        if let Some(rule) = self.rules.get_mut(&class) {
            rule.cooldown = match rule.cooldown.as_secs() {
                0..=29 => Duration::from_secs(30),
                30..=299 => Duration::from_secs(300),
                _ => Duration::ZERO,
            };
        }
    }
}

#[derive(Debug, Default)]
pub struct NotificationRateLimiter {
    last_sent: BTreeMap<(WorkspaceId, SignalClass), Timestamp>,
}

impl NotificationRateLimiter {
    pub fn should_send(
        &mut self,
        workspace_id: WorkspaceId,
        class: SignalClass,
        occurred_at: Timestamp,
        settings: &NotificationSettings,
    ) -> bool {
        let rule = settings.rule(class);
        if !rule.enabled {
            return false;
        }
        let key = (workspace_id, class);
        let cooldown = u64::try_from(rule.cooldown.as_millis()).unwrap_or(u64::MAX);
        if self.last_sent.get(&key).is_some_and(|last| {
            occurred_at
                .as_unix_millis()
                .saturating_sub(last.as_unix_millis())
                < cooldown
        }) {
            return false;
        }
        self.last_sent.insert(key, occurred_at);
        true
    }
}

pub fn notification_class(item: &TimelineItem) -> Option<SignalClass> {
    match item.attention() {
        AttentionLevel::None => None,
        AttentionLevel::Informational => Some(SignalClass::Completion),
        AttentionLevel::Urgent
            if item.title().to_ascii_lowercase().contains("fail")
                || item.detail().to_ascii_lowercase().contains("fail") =>
        {
            Some(SignalClass::Failure)
        }
        AttentionLevel::Urgent => Some(SignalClass::Attention),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        Agent, AgentId, Content, DomainCommand, Name, Task, TaskId, TimelineEvent, WorkspaceId,
    };
    use crate::timeline;

    fn workspace_with_task(workspace_id: u64, state: TaskState) -> (Workspace, Vec<TimelineItem>) {
        let workspace_id = WorkspaceId::new(workspace_id);
        let mut workspace = Workspace::new(workspace_id, Name::new("Project").unwrap());
        let task_id = TaskId::new(1);
        let added = workspace
            .execute(DomainCommand::AddTask(Task::new(
                task_id,
                Name::new("Work").unwrap(),
                Content::new("Do it").unwrap(),
                None,
                None,
            )))
            .unwrap();
        let transitions: &[TaskState] = match state {
            TaskState::Queued => &[],
            TaskState::Delivered => &[TaskState::Delivered],
            TaskState::Running => &[TaskState::Delivered, TaskState::Running],
            TaskState::Blocked => &[TaskState::Delivered, TaskState::Running, TaskState::Blocked],
            TaskState::Completed => &[
                TaskState::Delivered,
                TaskState::Running,
                TaskState::Completed,
            ],
            TaskState::Failed => &[TaskState::Failed],
            TaskState::Cancelled => &[TaskState::Cancelled],
        };
        let mut domain_events = vec![added];
        for to in transitions {
            domain_events.push(
                workspace
                    .execute(DomainCommand::TransitionTask { task_id, to: *to })
                    .unwrap(),
            );
        }
        let events = domain_events
            .into_iter()
            .enumerate()
            .map(|(index, event)| {
                TimelineEvent::new(
                    TimelineEventId::new(index as u64 + 1),
                    workspace_id,
                    Timestamp::from_unix_millis(index as u64 + 1),
                    event,
                )
            })
            .collect::<Vec<_>>();
        let items = timeline::project(&workspace, &events);
        (workspace, items)
    }

    fn workspace_with_failed_agent(workspace_id: u64) -> (Workspace, Vec<TimelineItem>) {
        let workspace_id = WorkspaceId::new(workspace_id);
        let mut workspace = Workspace::new(workspace_id, Name::new("Project").unwrap());
        let agent_id = AgentId::new(1);
        let added = workspace
            .execute(DomainCommand::AddAgent(Agent::new(
                agent_id,
                Name::new("Builder").unwrap(),
                None,
            )))
            .unwrap();
        let changed = workspace
            .execute(DomainCommand::TransitionAgent {
                agent_id,
                to: AgentState::Failed,
            })
            .unwrap();
        let events = [added, changed]
            .into_iter()
            .enumerate()
            .map(|(index, event)| {
                TimelineEvent::new(
                    TimelineEventId::new(index as u64 + 1),
                    workspace_id,
                    Timestamp::from_unix_millis(index as u64 + 1),
                    event,
                )
            })
            .collect::<Vec<_>>();
        let items = timeline::project(&workspace, &events);
        (workspace, items)
    }

    #[test]
    fn aggregates_workspaces_and_links_every_claim_to_evidence() {
        let (blocked, blocked_items) = workspace_with_task(1, TaskState::Blocked);
        let (completed, completed_items) = workspace_with_task(2, TaskState::Completed);
        let (failed, failed_items) = workspace_with_task(3, TaskState::Failed);
        let (failed_agent, failed_agent_items) = workspace_with_failed_agent(4);
        let snapshot = aggregate(
            [
                WorkspaceActivity {
                    workspace: &blocked,
                    timeline: &blocked_items,
                },
                WorkspaceActivity {
                    workspace: &completed,
                    timeline: &completed_items,
                },
                WorkspaceActivity {
                    workspace: &failed,
                    timeline: &failed_items,
                },
                WorkspaceActivity {
                    workspace: &failed_agent,
                    timeline: &failed_agent_items,
                },
            ],
            [CollisionObservation {
                workspace_id: WorkspaceId::new(1),
                path: "src/lib.rs".to_owned(),
                left_checkout: "main".to_owned(),
                right_checkout: "agent".to_owned(),
            }],
        );

        assert_eq!(snapshot.count(SignalClass::Attention), 1);
        assert_eq!(snapshot.count(SignalClass::Completion), 1);
        assert_eq!(snapshot.count(SignalClass::Failure), 2);
        assert_eq!(snapshot.count(SignalClass::Collision), 1);
        assert!(
            snapshot
                .claims()
                .iter()
                .all(|claim| !claim.evidence().is_empty())
        );
        assert_eq!(snapshot.recommendations().len(), snapshot.claims().len());
        assert!(
            snapshot
                .claims()
                .iter()
                .any(|claim| claim.text().contains("agent(s) failed"))
        );
    }

    #[test]
    fn duplicate_agent_names_keep_distinct_timeline_evidence() {
        let workspace_id = WorkspaceId::new(1);
        let mut workspace = Workspace::new(workspace_id, Name::new("Project").unwrap());
        let mut domain_events = Vec::new();
        for agent_id in [AgentId::new(1), AgentId::new(2)] {
            domain_events.push(
                workspace
                    .execute(DomainCommand::AddAgent(Agent::new(
                        agent_id,
                        Name::new("Builder").unwrap(),
                        None,
                    )))
                    .unwrap(),
            );
            domain_events.push(
                workspace
                    .execute(DomainCommand::TransitionAgent {
                        agent_id,
                        to: AgentState::Failed,
                    })
                    .unwrap(),
            );
        }
        let events = domain_events
            .into_iter()
            .enumerate()
            .map(|(index, event)| {
                TimelineEvent::new(
                    TimelineEventId::new(index as u64 + 1),
                    workspace_id,
                    Timestamp::from_unix_millis(index as u64 + 1),
                    event,
                )
            })
            .collect::<Vec<_>>();
        let items = timeline::project(&workspace, &events);

        let snapshot = aggregate(
            [WorkspaceActivity {
                workspace: &workspace,
                timeline: &items,
            }],
            [],
        );
        let event_ids = snapshot
            .claims()
            .iter()
            .filter(|claim| claim.text().contains("agent(s) failed"))
            .flat_map(Claim::evidence)
            .filter_map(|evidence| match evidence {
                Evidence::Timeline { event_id, .. } => Some(*event_id),
                Evidence::Collision { .. } => None,
            })
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(event_ids.len(), 2);
        assert!(event_ids.contains(&TimelineEventId::new(2)));
        assert!(event_ids.contains(&TimelineEventId::new(4)));
    }

    struct Adapter {
        residency: AdapterResidency,
        result: Result<String, String>,
    }

    impl SummaryAdapter for Adapter {
        fn residency(&self) -> AdapterResidency {
            self.residency
        }

        fn summarize(&self, _snapshot: &Snapshot) -> Result<String, String> {
            self.result.clone()
        }
    }

    #[test]
    fn adapter_privacy_and_failures_fall_back_without_losing_the_summary() {
        let snapshot = Snapshot::default();
        let external = Adapter {
            residency: AdapterResidency::External,
            result: Ok("remote".to_owned()),
        };
        let rejected = summarize(&snapshot, Some(&external), false);
        assert!(!rejected.used_adapter);
        assert!(rejected.adapter_error.is_some());

        let failing = Adapter {
            residency: AdapterResidency::OnDevice,
            result: Err("model unavailable".to_owned()),
        };
        let fallback = summarize(&snapshot, Some(&failing), false);
        assert!(!fallback.used_adapter);
        assert_eq!(fallback.text, rule_summary(&snapshot));
    }

    #[test]
    fn notification_limits_are_independent_by_workspace_and_class() {
        let mut settings = NotificationSettings::default();
        let mut limiter = NotificationRateLimiter::default();
        let at = Timestamp::from_unix_millis(1_000);

        assert!(limiter.should_send(WorkspaceId::new(1), SignalClass::Failure, at, &settings));
        assert!(!limiter.should_send(
            WorkspaceId::new(1),
            SignalClass::Failure,
            Timestamp::from_unix_millis(2_000),
            &settings
        ));
        assert!(limiter.should_send(WorkspaceId::new(2), SignalClass::Failure, at, &settings));
        assert!(limiter.should_send(WorkspaceId::new(1), SignalClass::Completion, at, &settings));

        settings.toggle(SignalClass::Attention);
        assert!(!limiter.should_send(WorkspaceId::new(3), SignalClass::Attention, at, &settings));
        settings.cycle_cooldown(SignalClass::Collision);
        settings.cycle_cooldown(SignalClass::Collision);
        assert!(limiter.should_send(WorkspaceId::new(3), SignalClass::Collision, at, &settings));
        assert!(limiter.should_send(WorkspaceId::new(3), SignalClass::Collision, at, &settings));
    }
}
