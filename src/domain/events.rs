use std::error::Error;
use std::fmt::{self, Display, Formatter};

use super::{
    Agent, AgentId, AgentState, Handoff, HandoffId, Node, NodeId, NodeTarget, Role, RoleId, Task,
    TaskId, TaskState, TimelineEventId, Timestamp, WorkspaceId,
};

#[derive(Debug, Clone, PartialEq)]
pub enum DomainCommand {
    AddRole(Role),
    AddAgent(Agent),
    AddTask(Task),
    AddHandoff(Handoff),
    AddNode(Node),
    TransitionAgent { agent_id: AgentId, to: AgentState },
    TransitionTask { task_id: TaskId, to: TaskState },
}

#[derive(Debug, Clone, PartialEq)]
pub enum DomainEvent {
    RoleAdded(Role),
    AgentAdded(Agent),
    TaskAdded(Task),
    HandoffAdded(Handoff),
    NodeAdded(Node),
    AgentStateChanged {
        agent_id: AgentId,
        from: AgentState,
        to: AgentState,
    },
    TaskStateChanged {
        task_id: TaskId,
        from: TaskState,
        to: TaskState,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimelineEvent {
    id: TimelineEventId,
    workspace_id: WorkspaceId,
    occurred_at: Timestamp,
    event: DomainEvent,
}

impl TimelineEvent {
    pub const fn new(
        id: TimelineEventId,
        workspace_id: WorkspaceId,
        occurred_at: Timestamp,
        event: DomainEvent,
    ) -> Self {
        Self {
            id,
            workspace_id,
            occurred_at,
            event,
        }
    }

    pub const fn id(&self) -> TimelineEventId {
        self.id
    }

    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub const fn occurred_at(&self) -> Timestamp {
        self.occurred_at
    }

    pub const fn event(&self) -> &DomainEvent {
        &self.event
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityRef {
    Workspace(WorkspaceId),
    Role(RoleId),
    Agent(AgentId),
    Task(TaskId),
    Handoff(HandoffId),
    Node(NodeId),
    TimelineEvent(TimelineEventId),
}

impl Display for EntityRef {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace(id) => write!(formatter, "workspace {id}"),
            Self::Role(id) => write!(formatter, "role {id}"),
            Self::Agent(id) => write!(formatter, "agent {id}"),
            Self::Task(id) => write!(formatter, "task {id}"),
            Self::Handoff(id) => write!(formatter, "handoff {id}"),
            Self::Node(id) => write!(formatter, "node {id}"),
            Self::TimelineEvent(id) => write!(formatter, "timeline event {id}"),
        }
    }
}

impl From<NodeTarget> for EntityRef {
    fn from(target: NodeTarget) -> Self {
        match target {
            NodeTarget::Agent(id) => Self::Agent(id),
            NodeTarget::Task(id) => Self::Task(id),
            NodeTarget::Handoff(id) => Self::Handoff(id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    DuplicateEntity(EntityRef),
    EntityNotFound(EntityRef),
    InvalidReference {
        entity: EntityRef,
        field: &'static str,
        target: EntityRef,
    },
    SameHandoffParticipant {
        handoff_id: HandoffId,
        agent_id: AgentId,
    },
    InvalidRetrySource {
        task_id: TaskId,
        retry_of: TaskId,
        state: TaskState,
    },
    InvalidAgentTransition {
        agent_id: AgentId,
        from: AgentState,
        to: AgentState,
    },
    InvalidTaskTransition {
        task_id: TaskId,
        from: TaskState,
        to: TaskState,
    },
    AgentStateConflict {
        agent_id: AgentId,
        expected: AgentState,
        actual: AgentState,
    },
    TaskStateConflict {
        task_id: TaskId,
        expected: TaskState,
        actual: TaskState,
    },
    WorkspaceMismatch {
        event_id: TimelineEventId,
        expected: WorkspaceId,
        actual: WorkspaceId,
    },
}

impl Display for DomainError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateEntity(entity) => write!(formatter, "{entity} already exists"),
            Self::EntityNotFound(entity) => write!(formatter, "{entity} does not exist"),
            Self::InvalidReference {
                entity,
                field,
                target,
            } => write!(formatter, "{entity} references missing {target} in {field}"),
            Self::SameHandoffParticipant {
                handoff_id,
                agent_id,
            } => write!(
                formatter,
                "handoff {handoff_id} must use different source and recipient agents; both are {agent_id}"
            ),
            Self::InvalidRetrySource {
                task_id,
                retry_of,
                state,
            } => write!(
                formatter,
                "task {task_id} cannot retry task {retry_of} while it is {state}; retry sources must be failed or cancelled"
            ),
            Self::InvalidAgentTransition { agent_id, from, to } => write!(
                formatter,
                "agent {agent_id} cannot transition from {from} to {to}"
            ),
            Self::InvalidTaskTransition { task_id, from, to } => write!(
                formatter,
                "task {task_id} cannot transition from {from} to {to}"
            ),
            Self::AgentStateConflict {
                agent_id,
                expected,
                actual,
            } => write!(
                formatter,
                "agent {agent_id} event expected state {expected}, but current state is {actual}"
            ),
            Self::TaskStateConflict {
                task_id,
                expected,
                actual,
            } => write!(
                formatter,
                "task {task_id} event expected state {expected}, but current state is {actual}"
            ),
            Self::WorkspaceMismatch {
                event_id,
                expected,
                actual,
            } => write!(
                formatter,
                "timeline event {event_id} belongs to workspace {actual}, not workspace {expected}"
            ),
        }
    }
}

impl Error for DomainError {}
