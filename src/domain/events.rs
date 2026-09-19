use std::error::Error;
use std::fmt::{self, Display, Formatter};

use super::{
    Agent, AgentId, AgentState, CanvasLayout, ConnectionId, EnvironmentProfile,
    EnvironmentProfileId, Handoff, HandoffId, Node, NodeGroupId, NodeId, NodeTarget, Role, RoleId,
    Task, TaskId, TaskState, TimelineEventId, Timestamp, WorkspaceId, WorkspaceSettings,
};

#[derive(Debug, Clone, PartialEq)]
pub enum DomainCommand {
    UpdateWorkspaceSettings(WorkspaceSettings),
    AddEnvironmentProfile(EnvironmentProfile),
    UpdateEnvironmentProfile(EnvironmentProfile),
    RemoveEnvironmentProfile(EnvironmentProfileId),
    AddRole(Role),
    AddAgent(Agent),
    AddTask(Task),
    AddHandoff(Handoff),
    AddNode(Node),
    AddAgentNode {
        agent: Agent,
        node: Node,
    },
    ReplaceCanvas {
        before: CanvasLayout,
        after: CanvasLayout,
    },
    TransitionAgent {
        agent_id: AgentId,
        to: AgentState,
    },
    TransitionTask {
        task_id: TaskId,
        to: TaskState,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum DomainEvent {
    WorkspaceSettingsChanged {
        from: WorkspaceSettings,
        to: WorkspaceSettings,
    },
    EnvironmentProfileAdded(EnvironmentProfile),
    EnvironmentProfileChanged {
        from: EnvironmentProfile,
        to: EnvironmentProfile,
    },
    EnvironmentProfileRemoved(EnvironmentProfile),
    RoleAdded(Role),
    AgentAdded(Agent),
    TaskAdded(Task),
    HandoffAdded(Handoff),
    NodeAdded(Node),
    AgentNodeAdded {
        agent: Agent,
        node: Node,
    },
    CanvasReplaced {
        before: CanvasLayout,
        after: CanvasLayout,
    },
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
    EnvironmentProfile(EnvironmentProfileId),
    Role(RoleId),
    Agent(AgentId),
    Task(TaskId),
    Handoff(HandoffId),
    Node(NodeId),
    NodeGroup(NodeGroupId),
    Connection(ConnectionId),
    TimelineEvent(TimelineEventId),
}

impl Display for EntityRef {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace(id) => write!(formatter, "workspace {id}"),
            Self::EnvironmentProfile(id) => write!(formatter, "environment profile {id}"),
            Self::Role(id) => write!(formatter, "role {id}"),
            Self::Agent(id) => write!(formatter, "agent {id}"),
            Self::Task(id) => write!(formatter, "task {id}"),
            Self::Handoff(id) => write!(formatter, "handoff {id}"),
            Self::Node(id) => write!(formatter, "node {id}"),
            Self::NodeGroup(id) => write!(formatter, "node group {id}"),
            Self::Connection(id) => write!(formatter, "connection {id}"),
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
    UnchangedWorkspaceSettings,
    WorkspaceSettingsConflict,
    UnchangedEnvironmentProfile,
    EnvironmentProfileConflict(EnvironmentProfileId),
    EnvironmentProfileInUse {
        profile_id: EnvironmentProfileId,
        agent_id: AgentId,
    },
    CanvasConflict,
    DuplicateEntity(EntityRef),
    EntityNotFound(EntityRef),
    InvalidReference {
        entity: EntityRef,
        field: &'static str,
        target: EntityRef,
    },
    InvalidGroup {
        group_id: NodeGroupId,
        detail: &'static str,
    },
    NodeInMultipleGroups {
        node_id: NodeId,
    },
    InvalidConnection {
        connection_id: ConnectionId,
        detail: &'static str,
    },
    DuplicateConnection {
        source: NodeId,
        target: NodeId,
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
            Self::UnchangedWorkspaceSettings => {
                formatter.write_str("workspace settings are unchanged")
            }
            Self::WorkspaceSettingsConflict => formatter.write_str(
                "workspace settings event does not match the current workspace settings",
            ),
            Self::UnchangedEnvironmentProfile => {
                formatter.write_str("environment profile is unchanged")
            }
            Self::EnvironmentProfileConflict(id) => write!(
                formatter,
                "environment profile {id} event does not match the current profile"
            ),
            Self::EnvironmentProfileInUse {
                profile_id,
                agent_id,
            } => write!(
                formatter,
                "environment profile {profile_id} is still used by agent {agent_id}"
            ),
            Self::CanvasConflict => {
                formatter.write_str("canvas edit does not match the current layout")
            }
            Self::DuplicateEntity(entity) => write!(formatter, "{entity} already exists"),
            Self::EntityNotFound(entity) => write!(formatter, "{entity} does not exist"),
            Self::InvalidReference {
                entity,
                field,
                target,
            } => write!(formatter, "{entity} references missing {target} in {field}"),
            Self::InvalidGroup { group_id, detail } => {
                write!(formatter, "node group {group_id} is invalid: {detail}")
            }
            Self::NodeInMultipleGroups { node_id } => {
                write!(formatter, "node {node_id} belongs to more than one group")
            }
            Self::InvalidConnection {
                connection_id,
                detail,
            } => write!(formatter, "connection {connection_id} is invalid: {detail}"),
            Self::DuplicateConnection { source, target } => write!(
                formatter,
                "a connection from node {source} to node {target} already exists"
            ),
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
