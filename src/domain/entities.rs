use std::collections::BTreeSet;

use super::{
    AgentId, AgentState, CanvasNodeContent, CanvasPoint, CanvasSize, CommandPresetId, ConnectionId,
    Content, DeliveryAttempt, EnvironmentProfileId, HandoffId, HandoffMessageId, HandoffProgress,
    HandoffResponse, HandoffTermination, Name, NodeGroupId, NodeId, RoleColor, RoleIcon, RoleId,
    RoutineRunId, RoutineStepId, TaskId, TaskState, Timestamp,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Role {
    id: RoleId,
    name: Name,
    color: RoleColor,
    icon: RoleIcon,
    instructions: Content,
}

impl Role {
    pub fn new(id: RoleId, name: Name, instructions: Content) -> Self {
        Self {
            id,
            name,
            color: RoleColor::default(),
            icon: RoleIcon::default(),
            instructions,
        }
    }

    pub const fn with_appearance(
        id: RoleId,
        name: Name,
        color: RoleColor,
        icon: RoleIcon,
        instructions: Content,
    ) -> Self {
        Self {
            id,
            name,
            color,
            icon,
            instructions,
        }
    }

    pub const fn id(&self) -> RoleId {
        self.id
    }

    pub const fn name(&self) -> &Name {
        &self.name
    }

    pub const fn color(&self) -> &RoleColor {
        &self.color
    }

    pub const fn icon(&self) -> &RoleIcon {
        &self.icon
    }

    pub const fn instructions(&self) -> &Content {
        &self.instructions
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    id: AgentId,
    name: Name,
    role_id: Option<RoleId>,
    program: AgentProgram,
    environment_id: Option<EnvironmentProfileId>,
    state: AgentState,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AgentProgram {
    Codex,
    Claude,
    OpenCode,
    Custom(CommandPresetId),
    #[default]
    Shell,
}

impl AgentProgram {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude",
            Self::OpenCode => "OpenCode",
            Self::Custom(_) => "Custom",
            Self::Shell => "Shell",
        }
    }

    pub const fn executable(self) -> Option<&'static str> {
        match self {
            Self::Codex => Some("codex"),
            Self::Claude => Some("claude"),
            Self::OpenCode => Some("opencode"),
            Self::Custom(_) | Self::Shell => None,
        }
    }
}

impl Agent {
    pub const fn new(id: AgentId, name: Name, role_id: Option<RoleId>) -> Self {
        Self::with_program(id, name, role_id, AgentProgram::Shell)
    }

    pub const fn with_program(
        id: AgentId,
        name: Name,
        role_id: Option<RoleId>,
        program: AgentProgram,
    ) -> Self {
        Self {
            id,
            name,
            role_id,
            program,
            environment_id: None,
            state: AgentState::Starting,
        }
    }

    pub const fn in_environment(mut self, environment_id: EnvironmentProfileId) -> Self {
        self.environment_id = Some(environment_id);
        self
    }

    pub const fn id(&self) -> AgentId {
        self.id
    }

    pub const fn name(&self) -> &Name {
        &self.name
    }

    pub const fn role_id(&self) -> Option<RoleId> {
        self.role_id
    }

    pub const fn program(&self) -> AgentProgram {
        self.program
    }

    pub const fn environment_id(&self) -> Option<EnvironmentProfileId> {
        self.environment_id
    }

    pub const fn state(&self) -> AgentState {
        self.state
    }

    pub(super) const fn set_state(&mut self, state: AgentState) {
        self.state = state;
    }

    pub(super) const fn set_role_id(&mut self, role_id: Option<RoleId>) {
        self.role_id = role_id;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    id: TaskId,
    title: Name,
    prompt: Content,
    assignee: Option<AgentId>,
    retry_of: Option<TaskId>,
    state: TaskState,
}

impl Task {
    pub const fn new(
        id: TaskId,
        title: Name,
        prompt: Content,
        assignee: Option<AgentId>,
        retry_of: Option<TaskId>,
    ) -> Self {
        Self {
            id,
            title,
            prompt,
            assignee,
            retry_of,
            state: TaskState::Queued,
        }
    }

    pub const fn id(&self) -> TaskId {
        self.id
    }

    pub const fn title(&self) -> &Name {
        &self.title
    }

    pub const fn prompt(&self) -> &Content {
        &self.prompt
    }

    pub const fn assignee(&self) -> Option<AgentId> {
        self.assignee
    }

    pub const fn retry_of(&self) -> Option<TaskId> {
        self.retry_of
    }

    pub const fn state(&self) -> TaskState {
        self.state
    }

    pub(super) const fn set_state(&mut self, state: TaskState) {
        self.state = state;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoffPayload {
    Task(TaskId),
    Question(Content),
}

/// Who submitted a handoff. Agents authenticate over IPC; the routine
/// scheduler submits internally and has no agent identity of its own, so it is
/// named explicitly rather than borrowing some agent's credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffOrigin {
    Agent(AgentId),
    Routine {
        run_id: RoutineRunId,
        step_id: RoutineStepId,
    },
}

impl HandoffOrigin {
    pub const fn agent(self) -> Option<AgentId> {
        match self {
            Self::Agent(agent_id) => Some(agent_id),
            Self::Routine { .. } => None,
        }
    }

    pub const fn run(self) -> Option<(RoutineRunId, RoutineStepId)> {
        match self {
            Self::Routine { run_id, step_id } => Some((run_id, step_id)),
            Self::Agent(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    pub(super) id: HandoffId,
    pub(super) origin: HandoffOrigin,
    pub(super) recipient: AgentId,
    pub(super) payload: HandoffPayload,
    pub(super) message_id: Option<HandoffMessageId>,
    pub(super) parent: Option<HandoffId>,
    pub(super) created_at: Option<Timestamp>,
    pub(super) response_deadline: Option<Timestamp>,
    pub(super) delivery_attempts: Vec<DeliveryAttempt>,
    pub(super) progress: Vec<HandoffProgress>,
    pub(super) response: Option<HandoffResponse>,
    pub(super) termination: Option<HandoffTermination>,
}

impl Handoff {
    pub const fn new(
        id: HandoffId,
        source: AgentId,
        recipient: AgentId,
        payload: HandoffPayload,
    ) -> Self {
        Self::with_origin(id, HandoffOrigin::Agent(source), recipient, payload)
    }

    pub const fn with_origin(
        id: HandoffId,
        origin: HandoffOrigin,
        recipient: AgentId,
        payload: HandoffPayload,
    ) -> Self {
        Self {
            id,
            origin,
            recipient,
            payload,
            message_id: None,
            parent: None,
            created_at: None,
            response_deadline: None,
            delivery_attempts: Vec::new(),
            progress: Vec::new(),
            response: None,
            termination: None,
        }
    }

    pub const fn id(&self) -> HandoffId {
        self.id
    }

    pub const fn origin(&self) -> HandoffOrigin {
        self.origin
    }

    /// The submitting agent, or `None` when the routine scheduler submitted it.
    pub const fn source(&self) -> Option<AgentId> {
        self.origin.agent()
    }

    pub const fn recipient(&self) -> AgentId {
        self.recipient
    }

    pub const fn payload(&self) -> &HandoffPayload {
        &self.payload
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeTarget {
    Agent(AgentId),
    Task(TaskId),
    Handoff(HandoffId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    id: NodeId,
    content: CanvasNodeContent,
    position: CanvasPoint,
    size: CanvasSize,
    z_index: i32,
}

impl Node {
    pub const fn new(
        id: NodeId,
        target: NodeTarget,
        position: CanvasPoint,
        size: CanvasSize,
    ) -> Self {
        Self::with_content(id, CanvasNodeContent::Reference(target), position, size)
    }

    pub const fn with_content(
        id: NodeId,
        content: CanvasNodeContent,
        position: CanvasPoint,
        size: CanvasSize,
    ) -> Self {
        Self {
            id,
            content,
            position,
            size,
            z_index: 0,
        }
    }

    pub const fn with_z_index(
        id: NodeId,
        target: NodeTarget,
        position: CanvasPoint,
        size: CanvasSize,
        z_index: i32,
    ) -> Self {
        Self::with_content_and_z_index(
            id,
            CanvasNodeContent::Reference(target),
            position,
            size,
            z_index,
        )
    }

    pub const fn with_content_and_z_index(
        id: NodeId,
        content: CanvasNodeContent,
        position: CanvasPoint,
        size: CanvasSize,
        z_index: i32,
    ) -> Self {
        Self {
            id,
            content,
            position,
            size,
            z_index,
        }
    }

    pub const fn id(&self) -> NodeId {
        self.id
    }

    pub const fn reference(&self) -> Option<NodeTarget> {
        self.content.reference()
    }

    pub const fn content(&self) -> &CanvasNodeContent {
        &self.content
    }

    pub const fn position(&self) -> CanvasPoint {
        self.position
    }

    pub const fn size(&self) -> CanvasSize {
        self.size
    }

    pub const fn z_index(&self) -> i32 {
        self.z_index
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeGroup {
    id: NodeGroupId,
    members: BTreeSet<NodeId>,
}

impl NodeGroup {
    pub fn new(id: NodeGroupId, members: impl IntoIterator<Item = NodeId>) -> Self {
        Self {
            id,
            members: members.into_iter().collect(),
        }
    }

    pub const fn id(&self) -> NodeGroupId {
        self.id
    }

    pub fn members(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.members.iter().copied()
    }

    pub fn contains(&self, node_id: NodeId) -> bool {
        self.members.contains(&node_id)
    }

    pub fn len(&self) -> usize {
        self.members.len()
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionKind {
    Coordination,
    Assignment,
    Dependency,
    Handoff,
    Reference,
}

impl ConnectionKind {
    pub const fn between(source: NodeTarget, target: NodeTarget) -> Option<Self> {
        match (source, target) {
            (NodeTarget::Agent(_), NodeTarget::Agent(_)) => Some(Self::Coordination),
            (NodeTarget::Agent(_), NodeTarget::Task(_)) => Some(Self::Assignment),
            (NodeTarget::Task(_), NodeTarget::Task(_)) => Some(Self::Dependency),
            (
                NodeTarget::Agent(_) | NodeTarget::Task(_) | NodeTarget::Handoff(_),
                NodeTarget::Handoff(_),
            )
            | (NodeTarget::Handoff(_), NodeTarget::Agent(_) | NodeTarget::Task(_)) => {
                Some(Self::Handoff)
            }
            _ => None,
        }
    }

    pub const fn between_content(
        source: &CanvasNodeContent,
        target: &CanvasNodeContent,
    ) -> Option<Self> {
        match (source.reference(), target.reference()) {
            (Some(source), Some(target)) => Self::between(source, target),
            (None, _) | (_, None) => Some(Self::Reference),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    id: ConnectionId,
    source: NodeId,
    target: NodeId,
    kind: ConnectionKind,
}

impl Connection {
    pub const fn new(
        id: ConnectionId,
        source: NodeId,
        target: NodeId,
        kind: ConnectionKind,
    ) -> Self {
        Self {
            id,
            source,
            target,
            kind,
        }
    }

    pub const fn id(&self) -> ConnectionId {
        self.id
    }

    pub const fn source(&self) -> NodeId {
        self.source
    }

    pub const fn target(&self) -> NodeId {
        self.target
    }

    pub const fn kind(&self) -> ConnectionKind {
        self.kind
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CanvasLayout {
    nodes: Vec<Node>,
    groups: Vec<NodeGroup>,
    connections: Vec<Connection>,
}

impl CanvasLayout {
    pub fn new(
        mut nodes: Vec<Node>,
        mut groups: Vec<NodeGroup>,
        mut connections: Vec<Connection>,
    ) -> Self {
        nodes.sort_by_key(Node::id);
        groups.sort_by_key(NodeGroup::id);
        connections.sort_by_key(Connection::id);
        Self {
            nodes,
            groups,
            connections,
        }
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn groups(&self) -> &[NodeGroup] {
        &self.groups
    }

    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }
}
