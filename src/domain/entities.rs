use std::collections::BTreeSet;

use super::{
    AgentId, AgentState, CanvasPoint, CanvasSize, ConnectionId, Content, EnvironmentProfileId,
    HandoffId, Name, NodeGroupId, NodeId, RoleId, TaskId, TaskState,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Role {
    id: RoleId,
    name: Name,
    instructions: Content,
}

impl Role {
    pub const fn new(id: RoleId, name: Name, instructions: Content) -> Self {
        Self {
            id,
            name,
            instructions,
        }
    }

    pub const fn id(&self) -> RoleId {
        self.id
    }

    pub const fn name(&self) -> &Name {
        &self.name
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
    #[default]
    Shell,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    id: HandoffId,
    source: AgentId,
    recipient: AgentId,
    payload: HandoffPayload,
}

impl Handoff {
    pub const fn new(
        id: HandoffId,
        source: AgentId,
        recipient: AgentId,
        payload: HandoffPayload,
    ) -> Self {
        Self {
            id,
            source,
            recipient,
            payload,
        }
    }

    pub const fn id(&self) -> HandoffId {
        self.id
    }

    pub const fn source(&self) -> AgentId {
        self.source
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
    target: NodeTarget,
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
        Self {
            id,
            target,
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
        Self {
            id,
            target,
            position,
            size,
            z_index,
        }
    }

    pub const fn id(&self) -> NodeId {
        self.id
    }

    pub const fn target(&self) -> NodeTarget {
        self.target
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
