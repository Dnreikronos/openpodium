use std::error::Error;
use std::fmt::{self, Display, Formatter};

use super::{
    Agent, AgentId, AgentState, CanvasLayout, ChatAttachment, ChatAttachmentId, ChatDraft,
    ChatMessage, ChatMessageId, ChatThread, ChatThreadId, ChatValidationError, CommandPreset,
    CommandPresetId, ConnectionId, EnvironmentProfile, EnvironmentProfileId, Handoff, HandoffId,
    HandoffMutationError, Name, Node, NodeGroupId, NodeId, NodeTarget, Role, RoleId, Task, TaskId,
    TaskState, ThreadColor, TimelineEventId, Timestamp, WorkspaceId, WorkspaceSettings,
};

#[derive(Debug, Clone, PartialEq)]
pub enum DomainCommand {
    ReplaceFloors {
        before: super::Floors,
        after: super::Floors,
    },
    UpdateWorkspaceSettings(WorkspaceSettings),
    AddEnvironmentProfile(EnvironmentProfile),
    UpdateEnvironmentProfile(EnvironmentProfile),
    RemoveEnvironmentProfile(EnvironmentProfileId),
    AddCommandPreset(CommandPreset),
    UpdateCommandPreset(CommandPreset),
    RemoveCommandPreset(CommandPresetId),
    AddRole(Role),
    UpdateRole(Role),
    RemoveRole(RoleId),
    AssignAgentRole {
        agent_id: AgentId,
        role_id: Option<RoleId>,
    },
    AddAgent(Agent),
    AddChatThread(ChatThread),
    UpdateChatThread {
        thread_id: ChatThreadId,
        name: Name,
        color: ThreadColor,
    },
    AddChatAttachment(ChatAttachment),
    UpdateChatDraft {
        thread_id: ChatThreadId,
        draft: ChatDraft,
    },
    SubmitChatDraft {
        thread_id: ChatThreadId,
        message_id: ChatMessageId,
        sent_at: Timestamp,
    },
    AppendAgentChatMessage(ChatMessage),
    AddTask(Task),
    AddHandoff(Handoff),
    AddTaskHandoff {
        task: Task,
        handoff: Handoff,
    },
    UpdateHandoff {
        before: Handoff,
        after: Handoff,
    },
    CancelTask {
        task_id: TaskId,
        before: Handoff,
        after: Handoff,
    },
    ResumeTask {
        task_id: TaskId,
        handoff: Handoff,
    },
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
    FloorsChanged {
        before: super::Floors,
        after: super::Floors,
    },
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
    CommandPresetAdded(CommandPreset),
    CommandPresetChanged {
        from: CommandPreset,
        to: CommandPreset,
    },
    CommandPresetRemoved(CommandPreset),
    RoleAdded(Role),
    RoleChanged {
        from: Role,
        to: Role,
    },
    RoleRemoved(Role),
    AgentRoleChanged {
        agent_id: AgentId,
        from: Option<RoleId>,
        to: Option<RoleId>,
    },
    AgentAdded(Agent),
    ChatThreadAdded(ChatThread),
    ChatThreadChanged {
        thread_id: ChatThreadId,
        from_name: Name,
        to_name: Name,
        from_color: ThreadColor,
        to_color: ThreadColor,
    },
    ChatAttachmentAdded(ChatAttachment),
    ChatDraftChanged {
        thread_id: ChatThreadId,
        from: ChatDraft,
        to: ChatDraft,
    },
    ChatDraftSubmitted {
        thread_id: ChatThreadId,
        from: ChatDraft,
        message: ChatMessage,
    },
    AgentChatMessageAppended(ChatMessage),
    TaskAdded(Task),
    HandoffAdded(Handoff),
    TaskHandoffAdded {
        task: Task,
        handoff: Handoff,
    },
    HandoffChanged {
        before: Handoff,
        after: Handoff,
    },
    TaskCancelled {
        task_id: TaskId,
        from: TaskState,
        before: Handoff,
        after: Handoff,
    },
    TaskResumed {
        task_id: TaskId,
        from: TaskState,
        handoff: Handoff,
    },
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
    CommandPreset(CommandPresetId),
    Role(RoleId),
    Agent(AgentId),
    ChatThread(ChatThreadId),
    ChatMessage(ChatMessageId),
    ChatAttachment(ChatAttachmentId),
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
            Self::CommandPreset(id) => write!(formatter, "command preset {id}"),
            Self::Role(id) => write!(formatter, "role {id}"),
            Self::Agent(id) => write!(formatter, "agent {id}"),
            Self::ChatThread(id) => write!(formatter, "chat thread {id}"),
            Self::ChatMessage(id) => write!(formatter, "chat message {id}"),
            Self::ChatAttachment(id) => write!(formatter, "chat attachment {id}"),
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
    FloorConflict,
    UnchangedWorkspaceSettings,
    WorkspaceSettingsConflict,
    UnchangedEnvironmentProfile,
    EnvironmentProfileConflict(EnvironmentProfileId),
    EnvironmentProfileInUse {
        profile_id: EnvironmentProfileId,
        agent_id: AgentId,
    },
    UnchangedCommandPreset,
    CommandPresetConflict(CommandPresetId),
    CommandPresetInUse {
        preset_id: CommandPresetId,
        agent_id: AgentId,
    },
    UnchangedRole,
    RoleConflict(RoleId),
    RoleInUse {
        role_id: RoleId,
        agent_id: AgentId,
    },
    UnchangedAgentRole,
    UnchangedChatThread,
    UnchangedChatDraft,
    ChatThreadConflict(ChatThreadId),
    ChatDraftConflict(ChatThreadId),
    ChatMessageConflict(ChatMessageId),
    ChatAttachmentWrongThread {
        attachment_id: ChatAttachmentId,
        expected: ChatThreadId,
        actual: ChatThreadId,
    },
    ChatAttachmentTotalTooLarge {
        thread_id: ChatThreadId,
        max_bytes: u64,
        actual_bytes: u64,
    },
    InvalidChatAuthor {
        message_id: ChatMessageId,
        expected: AgentId,
    },
    MentionNotConnected {
        thread_id: ChatThreadId,
        target: NodeTarget,
    },
    InvalidChat(ChatValidationError),
    AgentRoleConflict {
        agent_id: AgentId,
        expected: Option<RoleId>,
        actual: Option<RoleId>,
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
    DuplicateHandoffMessageId,
    HandoffChainTooDeep {
        handoff_id: HandoffId,
        max_depth: usize,
    },
    TaskHandoffMismatch {
        handoff_id: HandoffId,
        task_id: TaskId,
    },
    HandoffConflict(HandoffId),
    InvalidHandoff(HandoffMutationError),
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
            Self::FloorConflict => formatter.write_str("floor state conflicts with the workspace"),
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
            Self::UnchangedCommandPreset => formatter.write_str("command preset is unchanged"),
            Self::CommandPresetConflict(id) => write!(
                formatter,
                "command preset {id} event does not match the current preset"
            ),
            Self::CommandPresetInUse {
                preset_id,
                agent_id,
            } => write!(
                formatter,
                "command preset {preset_id} is still used by agent {agent_id}"
            ),
            Self::UnchangedRole => formatter.write_str("role is unchanged"),
            Self::RoleConflict(id) => {
                write!(formatter, "role {id} event does not match the current role")
            }
            Self::RoleInUse { role_id, agent_id } => {
                write!(
                    formatter,
                    "role {role_id} is still used by agent {agent_id}"
                )
            }
            Self::UnchangedAgentRole => formatter.write_str("agent role is unchanged"),
            Self::UnchangedChatThread => formatter.write_str("chat thread is unchanged"),
            Self::UnchangedChatDraft => formatter.write_str("chat draft is unchanged"),
            Self::ChatThreadConflict(id) => {
                write!(
                    formatter,
                    "chat thread {id} event does not match the current thread"
                )
            }
            Self::ChatDraftConflict(id) => {
                write!(formatter, "chat draft event does not match thread {id}")
            }
            Self::ChatMessageConflict(id) => {
                write!(formatter, "chat message {id} does not match its draft")
            }
            Self::ChatAttachmentWrongThread {
                attachment_id,
                expected,
                actual,
            } => write!(
                formatter,
                "chat attachment {attachment_id} belongs to thread {actual}, not thread {expected}"
            ),
            Self::ChatAttachmentTotalTooLarge {
                thread_id,
                max_bytes,
                actual_bytes,
            } => write!(
                formatter,
                "chat draft for thread {thread_id} attaches {actual_bytes} bytes; the limit is {max_bytes} bytes"
            ),
            Self::InvalidChatAuthor {
                message_id,
                expected,
            } => write!(
                formatter,
                "chat message {message_id} must be authored by agent {expected}"
            ),
            Self::MentionNotConnected { thread_id, target } => write!(
                formatter,
                "chat thread {thread_id} cannot mention unconnected target {target:?}"
            ),
            Self::InvalidChat(error) => error.fmt(formatter),
            Self::AgentRoleConflict {
                agent_id,
                expected,
                actual,
            } => write!(
                formatter,
                "agent {agent_id} role event expected {expected:?}, but current role is {actual:?}"
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
            Self::DuplicateHandoffMessageId => {
                formatter.write_str("handoff message ID already exists in this workspace")
            }
            Self::HandoffChainTooDeep {
                handoff_id,
                max_depth,
            } => write!(
                formatter,
                "handoff {handoff_id} exceeds the maximum parent depth of {max_depth}"
            ),
            Self::TaskHandoffMismatch {
                handoff_id,
                task_id,
            } => write!(
                formatter,
                "handoff {handoff_id} does not reference its task {task_id} or recipient"
            ),
            Self::HandoffConflict(id) => {
                write!(
                    formatter,
                    "handoff {id} event does not match the current handoff"
                )
            }
            Self::InvalidHandoff(error) => error.fmt(formatter),
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

impl From<ChatValidationError> for DomainError {
    fn from(error: ChatValidationError) -> Self {
        Self::InvalidChat(error)
    }
}

impl From<HandoffMutationError> for DomainError {
    fn from(error: HandoffMutationError) -> Self {
        Self::InvalidHandoff(error)
    }
}
