use super::{
    AgentId, AgentState, CanvasPoint, CanvasSize, Content, HandoffId, Name, NodeId, RoleId, TaskId,
    TaskState,
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
    state: AgentState,
}

impl Agent {
    pub const fn new(id: AgentId, name: Name, role_id: Option<RoleId>) -> Self {
        Self {
            id,
            name,
            role_id,
            state: AgentState::Starting,
        }
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
}
