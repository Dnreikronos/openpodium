use serde::{Deserialize, Serialize};

use crate::domain::{
    Agent, AgentId, AgentProgram, AgentState, CanvasLayout, CanvasPoint, CanvasSize, Connection,
    ConnectionId, ConnectionKind, Content, DomainCommand, DomainEvent, Handoff, HandoffId,
    HandoffPayload, Name, Node, NodeGroup, NodeGroupId, NodeId, NodeTarget, Role, RoleId, Task,
    TaskId, TaskState, Workspace, WorkspaceDirectory, WorkspaceIcon, WorkspaceId,
    WorkspaceSettings,
};

use super::PersistenceError;

pub(crate) const EVENT_FORMAT_VERSION: u32 = 3;
pub(crate) const SNAPSHOT_FORMAT_VERSION: u32 = 3;

pub(crate) fn encode_event(event: &DomainEvent) -> Result<Vec<u8>, PersistenceError> {
    serde_json::to_vec(&StoredEvent::from(event)).map_err(|source| {
        PersistenceError::Serialization {
            record_type: "domain event",
            source,
        }
    })
}

pub(crate) fn decode_event(
    payload: &[u8],
    sequence: u64,
    format_version: u32,
) -> Result<DomainEvent, PersistenceError> {
    let stored: StoredEvent =
        serde_json::from_slice(payload).map_err(|source| PersistenceError::Deserialization {
            record_type: "domain event",
            sequence,
            source,
        })?;

    if format_version == 1 && matches!(stored, StoredEvent::WorkspaceSettingsChanged { .. }) {
        return Err(PersistenceError::invalid_record(
            "domain event",
            sequence,
            "workspace settings changes require event format version 2",
        ));
    }
    if format_version < 3 && stored.requires_version_three() {
        return Err(PersistenceError::invalid_record(
            "domain event",
            sequence,
            "canvas graph edits and agent programs require event format version 3",
        ));
    }

    stored
        .into_domain()
        .map_err(|detail| PersistenceError::invalid_record("domain event", sequence, detail))
}

pub(crate) fn encode_workspace(workspace: &Workspace) -> Result<Vec<u8>, PersistenceError> {
    serde_json::to_vec(&StoredWorkspace::from(workspace)).map_err(|source| {
        PersistenceError::Serialization {
            record_type: "workspace snapshot",
            source,
        }
    })
}

pub(crate) fn decode_workspace(
    payload: &[u8],
    sequence: u64,
    format_version: u32,
) -> Result<Workspace, PersistenceError> {
    let stored: StoredWorkspace =
        serde_json::from_slice(payload).map_err(|source| PersistenceError::Deserialization {
            record_type: "workspace snapshot",
            sequence,
            source,
        })?;

    stored
        .into_domain(format_version)
        .map_err(|detail| PersistenceError::invalid_record("workspace snapshot", sequence, detail))
}

pub(crate) fn checksum(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    for part in parts {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    *hasher.finalize().as_bytes()
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum StoredEvent {
    WorkspaceSettingsChanged {
        from: StoredWorkspaceSettings,
        to: StoredWorkspaceSettings,
    },
    RoleAdded {
        role: RoleV1,
    },
    AgentAdded {
        agent: AgentV1,
    },
    TaskAdded {
        task: TaskV1,
    },
    HandoffAdded {
        handoff: HandoffV1,
    },
    NodeAdded {
        node: NodeV1,
    },
    AgentNodeAdded {
        agent: AgentV1,
        node: NodeV1,
    },
    CanvasReplaced {
        before: CanvasLayoutV1,
        after: CanvasLayoutV1,
    },
    AgentStateChanged {
        agent_id: u64,
        from: AgentStateV1,
        to: AgentStateV1,
    },
    TaskStateChanged {
        task_id: u64,
        from: TaskStateV1,
        to: TaskStateV1,
    },
}

impl From<&DomainEvent> for StoredEvent {
    fn from(event: &DomainEvent) -> Self {
        match event {
            DomainEvent::WorkspaceSettingsChanged { from, to } => Self::WorkspaceSettingsChanged {
                from: StoredWorkspaceSettings::from(from),
                to: StoredWorkspaceSettings::from(to),
            },
            DomainEvent::RoleAdded(role) => Self::RoleAdded {
                role: RoleV1::from(role),
            },
            DomainEvent::AgentAdded(agent) => Self::AgentAdded {
                agent: AgentV1::from(agent),
            },
            DomainEvent::TaskAdded(task) => Self::TaskAdded {
                task: TaskV1::from(task),
            },
            DomainEvent::HandoffAdded(handoff) => Self::HandoffAdded {
                handoff: HandoffV1::from(handoff),
            },
            DomainEvent::NodeAdded(node) => Self::NodeAdded {
                node: NodeV1::from(node),
            },
            DomainEvent::AgentNodeAdded { agent, node } => Self::AgentNodeAdded {
                agent: AgentV1::from(agent),
                node: NodeV1::from(node),
            },
            DomainEvent::CanvasReplaced { before, after } => Self::CanvasReplaced {
                before: CanvasLayoutV1::from(before),
                after: CanvasLayoutV1::from(after),
            },
            DomainEvent::AgentStateChanged { agent_id, from, to } => Self::AgentStateChanged {
                agent_id: agent_id.get(),
                from: (*from).into(),
                to: (*to).into(),
            },
            DomainEvent::TaskStateChanged { task_id, from, to } => Self::TaskStateChanged {
                task_id: task_id.get(),
                from: (*from).into(),
                to: (*to).into(),
            },
        }
    }
}

impl StoredEvent {
    fn requires_version_three(&self) -> bool {
        match self {
            Self::AgentAdded { agent } => agent.program != AgentProgramV1::Shell,
            Self::NodeAdded { node } => node.z_index != 0,
            Self::AgentNodeAdded { .. } | Self::CanvasReplaced { .. } => true,
            _ => false,
        }
    }

    fn into_domain(self) -> Result<DomainEvent, String> {
        match self {
            Self::WorkspaceSettingsChanged { from, to } => {
                Ok(DomainEvent::WorkspaceSettingsChanged {
                    from: from.into_domain()?,
                    to: to.into_domain()?,
                })
            }
            Self::RoleAdded { role } => Ok(DomainEvent::RoleAdded(role.into_domain()?)),
            Self::AgentAdded { agent } => {
                let (agent, state) = agent.into_domain()?;
                if state != AgentState::Starting {
                    return Err("an agent_added event must contain a starting agent".to_owned());
                }
                Ok(DomainEvent::AgentAdded(agent))
            }
            Self::TaskAdded { task } => {
                let (task, state) = task.into_domain()?;
                if state != TaskState::Queued {
                    return Err("a task_added event must contain a queued task".to_owned());
                }
                Ok(DomainEvent::TaskAdded(task))
            }
            Self::HandoffAdded { handoff } => Ok(DomainEvent::HandoffAdded(handoff.into_domain()?)),
            Self::NodeAdded { node } => Ok(DomainEvent::NodeAdded(node.into_domain()?)),
            Self::AgentNodeAdded { agent, node } => {
                let (agent, state) = agent.into_domain()?;
                if state != AgentState::Starting {
                    return Err(
                        "an agent_node_added event must contain a starting agent".to_owned()
                    );
                }
                Ok(DomainEvent::AgentNodeAdded {
                    agent,
                    node: node.into_domain()?,
                })
            }
            Self::CanvasReplaced { before, after } => Ok(DomainEvent::CanvasReplaced {
                before: before.into_domain()?,
                after: after.into_domain()?,
            }),
            Self::AgentStateChanged { agent_id, from, to } => Ok(DomainEvent::AgentStateChanged {
                agent_id: AgentId::new(agent_id),
                from: from.into(),
                to: to.into(),
            }),
            Self::TaskStateChanged { task_id, from, to } => Ok(DomainEvent::TaskStateChanged {
                task_id: TaskId::new(task_id),
                from: from.into(),
                to: to.into(),
            }),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredWorkspace {
    id: u64,
    name: String,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    working_directory: Option<String>,
    #[serde(default)]
    instructions: Option<String>,
    roles: Vec<RoleV1>,
    agents: Vec<AgentV1>,
    tasks: Vec<TaskV1>,
    handoffs: Vec<HandoffV1>,
    nodes: Vec<NodeV1>,
    #[serde(default)]
    groups: Vec<NodeGroupV1>,
    #[serde(default)]
    connections: Vec<ConnectionV1>,
}

impl From<&Workspace> for StoredWorkspace {
    fn from(workspace: &Workspace) -> Self {
        Self {
            id: workspace.id().get(),
            name: workspace.name().to_owned(),
            icon: workspace
                .settings()
                .icon()
                .map(|icon| icon.as_str().to_owned()),
            working_directory: workspace
                .settings()
                .working_directory()
                .map(|directory| directory.as_str().to_owned()),
            instructions: workspace
                .settings()
                .instructions()
                .map(|instructions| instructions.as_str().to_owned()),
            roles: workspace.roles().map(RoleV1::from).collect(),
            agents: workspace.agents().map(AgentV1::from).collect(),
            tasks: workspace.tasks().map(TaskV1::from).collect(),
            handoffs: workspace.handoffs().map(HandoffV1::from).collect(),
            nodes: workspace.nodes().map(NodeV1::from).collect(),
            groups: workspace.groups().map(NodeGroupV1::from).collect(),
            connections: workspace.connections().map(ConnectionV1::from).collect(),
        }
    }
}

impl StoredWorkspace {
    fn into_domain(self, format_version: u32) -> Result<Workspace, String> {
        if format_version == 1
            && (self.icon.is_some()
                || self.working_directory.is_some()
                || self.instructions.is_some())
        {
            return Err("workspace settings require snapshot format version 2".to_owned());
        }
        if format_version < 3
            && (self
                .agents
                .iter()
                .any(|agent| agent.program != AgentProgramV1::Shell)
                || self.nodes.iter().any(|node| node.z_index != 0)
                || !self.groups.is_empty()
                || !self.connections.is_empty())
        {
            return Err(
                "canvas graph state and agent programs require snapshot format version 3"
                    .to_owned(),
            );
        }

        let settings = StoredWorkspaceSettings {
            name: self.name,
            icon: self.icon,
            working_directory: self.working_directory,
            instructions: self.instructions,
        }
        .into_domain()?;
        let mut workspace = Workspace::new(WorkspaceId::new(self.id), settings.name().clone());
        if workspace.settings() != &settings {
            apply_snapshot_command(
                &mut workspace,
                DomainCommand::UpdateWorkspaceSettings(settings),
            )?;
        }

        for role in self.roles {
            apply_snapshot_command(&mut workspace, DomainCommand::AddRole(role.into_domain()?))?;
        }
        for agent in self.agents {
            let (agent, state) = agent.into_domain()?;
            let agent_id = agent.id();
            apply_snapshot_command(&mut workspace, DomainCommand::AddAgent(agent))?;
            restore_agent_state(&mut workspace, agent_id, state)?;
        }

        let mut pending_tasks = self.tasks;
        while !pending_tasks.is_empty() {
            let Some(index) = pending_tasks.iter().position(|task| {
                task.retry_of
                    .is_none_or(|retry_of| workspace.task(TaskId::new(retry_of)).is_some())
            }) else {
                return Err("task retry references contain a cycle or missing task".to_owned());
            };
            let (task, state) = pending_tasks.remove(index).into_domain()?;
            let task_id = task.id();
            apply_snapshot_command(&mut workspace, DomainCommand::AddTask(task))?;
            restore_task_state(&mut workspace, task_id, state)?;
        }

        for handoff in self.handoffs {
            apply_snapshot_command(
                &mut workspace,
                DomainCommand::AddHandoff(handoff.into_domain()?),
            )?;
        }
        for node in self.nodes {
            apply_snapshot_command(&mut workspace, DomainCommand::AddNode(node.into_domain()?))?;
        }
        let before = workspace.canvas_layout();
        let after = CanvasLayout::new(
            before.nodes().to_vec(),
            self.groups
                .into_iter()
                .map(NodeGroupV1::into_domain)
                .collect::<Result<_, _>>()?,
            self.connections
                .into_iter()
                .map(ConnectionV1::into_domain)
                .collect(),
        );
        if before != after {
            apply_snapshot_command(
                &mut workspace,
                DomainCommand::ReplaceCanvas { before, after },
            )?;
        }

        Ok(workspace)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredWorkspaceSettings {
    name: String,
    icon: Option<String>,
    working_directory: Option<String>,
    instructions: Option<String>,
}

impl From<&WorkspaceSettings> for StoredWorkspaceSettings {
    fn from(settings: &WorkspaceSettings) -> Self {
        Self {
            name: settings.name().as_str().to_owned(),
            icon: settings.icon().map(|icon| icon.as_str().to_owned()),
            working_directory: settings
                .working_directory()
                .map(|directory| directory.as_str().to_owned()),
            instructions: settings
                .instructions()
                .map(|instructions| instructions.as_str().to_owned()),
        }
    }
}

impl StoredWorkspaceSettings {
    fn into_domain(self) -> Result<WorkspaceSettings, String> {
        Ok(WorkspaceSettings::new(
            Name::new(self.name).map_err(|error| error.to_string())?,
            self.icon
                .map(WorkspaceIcon::new)
                .transpose()
                .map_err(|error| error.to_string())?,
            self.working_directory
                .map(WorkspaceDirectory::new)
                .transpose()
                .map_err(|error| error.to_string())?,
            self.instructions
                .map(Content::new)
                .transpose()
                .map_err(|error| error.to_string())?,
        ))
    }
}

fn restore_agent_state(
    workspace: &mut Workspace,
    agent_id: AgentId,
    state: AgentState,
) -> Result<(), String> {
    let transitions: &[AgentState] = match state {
        AgentState::Starting => &[],
        AgentState::Running => &[AgentState::Running],
        AgentState::Waiting => &[AgentState::Running, AgentState::Waiting],
        AgentState::Completed => &[AgentState::Running, AgentState::Completed],
        AgentState::Failed => &[AgentState::Failed],
        AgentState::Stopped => &[AgentState::Stopped],
    };
    for to in transitions {
        apply_snapshot_command(
            workspace,
            DomainCommand::TransitionAgent { agent_id, to: *to },
        )?;
    }
    Ok(())
}

fn restore_task_state(
    workspace: &mut Workspace,
    task_id: TaskId,
    state: TaskState,
) -> Result<(), String> {
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
        TaskState::Failed => &[TaskState::Delivered, TaskState::Failed],
        TaskState::Cancelled => &[TaskState::Cancelled],
    };
    for to in transitions {
        apply_snapshot_command(
            workspace,
            DomainCommand::TransitionTask { task_id, to: *to },
        )?;
    }
    Ok(())
}

fn apply_snapshot_command(workspace: &mut Workspace, command: DomainCommand) -> Result<(), String> {
    workspace
        .execute(command)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleV1 {
    id: u64,
    name: String,
    instructions: String,
}

impl From<&Role> for RoleV1 {
    fn from(role: &Role) -> Self {
        Self {
            id: role.id().get(),
            name: role.name().as_str().to_owned(),
            instructions: role.instructions().as_str().to_owned(),
        }
    }
}

impl RoleV1 {
    fn into_domain(self) -> Result<Role, String> {
        Ok(Role::new(
            RoleId::new(self.id),
            Name::new(self.name).map_err(|error| error.to_string())?,
            Content::new(self.instructions).map_err(|error| error.to_string())?,
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentV1 {
    id: u64,
    name: String,
    role_id: Option<u64>,
    #[serde(default)]
    program: AgentProgramV1,
    state: AgentStateV1,
}

impl From<&Agent> for AgentV1 {
    fn from(agent: &Agent) -> Self {
        Self {
            id: agent.id().get(),
            name: agent.name().as_str().to_owned(),
            role_id: agent.role_id().map(RoleId::get),
            program: agent.program().into(),
            state: agent.state().into(),
        }
    }
}

impl AgentV1 {
    fn into_domain(self) -> Result<(Agent, AgentState), String> {
        Ok((
            Agent::with_program(
                AgentId::new(self.id),
                Name::new(self.name).map_err(|error| error.to_string())?,
                self.role_id.map(RoleId::new),
                self.program.into(),
            ),
            self.state.into(),
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskV1 {
    id: u64,
    title: String,
    prompt: String,
    assignee: Option<u64>,
    retry_of: Option<u64>,
    state: TaskStateV1,
}

impl From<&Task> for TaskV1 {
    fn from(task: &Task) -> Self {
        Self {
            id: task.id().get(),
            title: task.title().as_str().to_owned(),
            prompt: task.prompt().as_str().to_owned(),
            assignee: task.assignee().map(AgentId::get),
            retry_of: task.retry_of().map(TaskId::get),
            state: task.state().into(),
        }
    }
}

impl TaskV1 {
    fn into_domain(self) -> Result<(Task, TaskState), String> {
        Ok((
            Task::new(
                TaskId::new(self.id),
                Name::new(self.title).map_err(|error| error.to_string())?,
                Content::new(self.prompt).map_err(|error| error.to_string())?,
                self.assignee.map(AgentId::new),
                self.retry_of.map(TaskId::new),
            ),
            self.state.into(),
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HandoffV1 {
    id: u64,
    source: u64,
    recipient: u64,
    payload: HandoffPayloadV1,
}

impl From<&Handoff> for HandoffV1 {
    fn from(handoff: &Handoff) -> Self {
        Self {
            id: handoff.id().get(),
            source: handoff.source().get(),
            recipient: handoff.recipient().get(),
            payload: HandoffPayloadV1::from(handoff.payload()),
        }
    }
}

impl HandoffV1 {
    fn into_domain(self) -> Result<Handoff, String> {
        Ok(Handoff::new(
            HandoffId::new(self.id),
            AgentId::new(self.source),
            AgentId::new(self.recipient),
            self.payload.into_domain()?,
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum HandoffPayloadV1 {
    Task { task_id: u64 },
    Question { content: String },
}

impl From<&HandoffPayload> for HandoffPayloadV1 {
    fn from(payload: &HandoffPayload) -> Self {
        match payload {
            HandoffPayload::Task(task_id) => Self::Task {
                task_id: task_id.get(),
            },
            HandoffPayload::Question(content) => Self::Question {
                content: content.as_str().to_owned(),
            },
        }
    }
}

impl HandoffPayloadV1 {
    fn into_domain(self) -> Result<HandoffPayload, String> {
        match self {
            Self::Task { task_id } => Ok(HandoffPayload::Task(TaskId::new(task_id))),
            Self::Question { content } => Ok(HandoffPayload::Question(
                Content::new(content).map_err(|error| error.to_string())?,
            )),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeV1 {
    id: u64,
    target: NodeTargetV1,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    #[serde(default)]
    z_index: i32,
}

impl From<&Node> for NodeV1 {
    fn from(node: &Node) -> Self {
        Self {
            id: node.id().get(),
            target: node.target().into(),
            x: node.position().x(),
            y: node.position().y(),
            width: node.size().width(),
            height: node.size().height(),
            z_index: node.z_index(),
        }
    }
}

impl NodeV1 {
    fn into_domain(self) -> Result<Node, String> {
        Ok(Node::with_z_index(
            NodeId::new(self.id),
            self.target.into(),
            CanvasPoint::new(self.x, self.y).map_err(|error| error.to_string())?,
            CanvasSize::new(self.width, self.height).map_err(|error| error.to_string())?,
            self.z_index,
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeGroupV1 {
    id: u64,
    members: Vec<u64>,
}

impl From<&NodeGroup> for NodeGroupV1 {
    fn from(group: &NodeGroup) -> Self {
        Self {
            id: group.id().get(),
            members: group.members().map(NodeId::get).collect(),
        }
    }
}

impl NodeGroupV1 {
    fn into_domain(self) -> Result<NodeGroup, String> {
        if self.members.len() < 2 {
            return Err("a stored node group must contain at least two members".to_owned());
        }
        Ok(NodeGroup::new(
            NodeGroupId::new(self.id),
            self.members.into_iter().map(NodeId::new),
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionV1 {
    id: u64,
    source: u64,
    target: u64,
    kind: ConnectionKindV1,
}

impl From<&Connection> for ConnectionV1 {
    fn from(connection: &Connection) -> Self {
        Self {
            id: connection.id().get(),
            source: connection.source().get(),
            target: connection.target().get(),
            kind: connection.kind().into(),
        }
    }
}

impl ConnectionV1 {
    fn into_domain(self) -> Connection {
        Connection::new(
            ConnectionId::new(self.id),
            NodeId::new(self.source),
            NodeId::new(self.target),
            self.kind.into(),
        )
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ConnectionKindV1 {
    Coordination,
    Assignment,
    Dependency,
    Handoff,
}

impl From<ConnectionKind> for ConnectionKindV1 {
    fn from(kind: ConnectionKind) -> Self {
        match kind {
            ConnectionKind::Coordination => Self::Coordination,
            ConnectionKind::Assignment => Self::Assignment,
            ConnectionKind::Dependency => Self::Dependency,
            ConnectionKind::Handoff => Self::Handoff,
        }
    }
}

impl From<ConnectionKindV1> for ConnectionKind {
    fn from(kind: ConnectionKindV1) -> Self {
        match kind {
            ConnectionKindV1::Coordination => Self::Coordination,
            ConnectionKindV1::Assignment => Self::Assignment,
            ConnectionKindV1::Dependency => Self::Dependency,
            ConnectionKindV1::Handoff => Self::Handoff,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CanvasLayoutV1 {
    nodes: Vec<NodeV1>,
    groups: Vec<NodeGroupV1>,
    connections: Vec<ConnectionV1>,
}

impl From<&CanvasLayout> for CanvasLayoutV1 {
    fn from(layout: &CanvasLayout) -> Self {
        Self {
            nodes: layout.nodes().iter().map(NodeV1::from).collect(),
            groups: layout.groups().iter().map(NodeGroupV1::from).collect(),
            connections: layout
                .connections()
                .iter()
                .map(ConnectionV1::from)
                .collect(),
        }
    }
}

impl CanvasLayoutV1 {
    fn into_domain(self) -> Result<CanvasLayout, String> {
        Ok(CanvasLayout::new(
            self.nodes
                .into_iter()
                .map(NodeV1::into_domain)
                .collect::<Result<_, _>>()?,
            self.groups
                .into_iter()
                .map(NodeGroupV1::into_domain)
                .collect::<Result<_, _>>()?,
            self.connections
                .into_iter()
                .map(ConnectionV1::into_domain)
                .collect(),
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum NodeTargetV1 {
    Agent { id: u64 },
    Task { id: u64 },
    Handoff { id: u64 },
}

impl From<NodeTarget> for NodeTargetV1 {
    fn from(target: NodeTarget) -> Self {
        match target {
            NodeTarget::Agent(id) => Self::Agent { id: id.get() },
            NodeTarget::Task(id) => Self::Task { id: id.get() },
            NodeTarget::Handoff(id) => Self::Handoff { id: id.get() },
        }
    }
}

impl From<NodeTargetV1> for NodeTarget {
    fn from(target: NodeTargetV1) -> Self {
        match target {
            NodeTargetV1::Agent { id } => Self::Agent(AgentId::new(id)),
            NodeTargetV1::Task { id } => Self::Task(TaskId::new(id)),
            NodeTargetV1::Handoff { id } => Self::Handoff(HandoffId::new(id)),
        }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AgentStateV1 {
    Starting,
    Running,
    Waiting,
    Completed,
    Failed,
    Stopped,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AgentProgramV1 {
    Codex,
    Claude,
    #[default]
    Shell,
}

impl From<AgentProgram> for AgentProgramV1 {
    fn from(program: AgentProgram) -> Self {
        match program {
            AgentProgram::Codex => Self::Codex,
            AgentProgram::Claude => Self::Claude,
            AgentProgram::Shell => Self::Shell,
        }
    }
}

impl From<AgentProgramV1> for AgentProgram {
    fn from(program: AgentProgramV1) -> Self {
        match program {
            AgentProgramV1::Codex => Self::Codex,
            AgentProgramV1::Claude => Self::Claude,
            AgentProgramV1::Shell => Self::Shell,
        }
    }
}

impl From<AgentState> for AgentStateV1 {
    fn from(state: AgentState) -> Self {
        match state {
            AgentState::Starting => Self::Starting,
            AgentState::Running => Self::Running,
            AgentState::Waiting => Self::Waiting,
            AgentState::Completed => Self::Completed,
            AgentState::Failed => Self::Failed,
            AgentState::Stopped => Self::Stopped,
        }
    }
}

impl From<AgentStateV1> for AgentState {
    fn from(state: AgentStateV1) -> Self {
        match state {
            AgentStateV1::Starting => Self::Starting,
            AgentStateV1::Running => Self::Running,
            AgentStateV1::Waiting => Self::Waiting,
            AgentStateV1::Completed => Self::Completed,
            AgentStateV1::Failed => Self::Failed,
            AgentStateV1::Stopped => Self::Stopped,
        }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TaskStateV1 {
    Queued,
    Delivered,
    Running,
    Blocked,
    Completed,
    Failed,
    Cancelled,
}

impl From<TaskState> for TaskStateV1 {
    fn from(state: TaskState) -> Self {
        match state {
            TaskState::Queued => Self::Queued,
            TaskState::Delivered => Self::Delivered,
            TaskState::Running => Self::Running,
            TaskState::Blocked => Self::Blocked,
            TaskState::Completed => Self::Completed,
            TaskState::Failed => Self::Failed,
            TaskState::Cancelled => Self::Cancelled,
        }
    }
}

impl From<TaskStateV1> for TaskState {
    fn from(state: TaskStateV1) -> Self {
        match state {
            TaskStateV1::Queued => Self::Queued,
            TaskStateV1::Delivered => Self::Delivered,
            TaskStateV1::Running => Self::Running,
            TaskStateV1::Blocked => Self::Blocked,
            TaskStateV1::Completed => Self::Completed,
            TaskStateV1::Failed => Self::Failed,
            TaskStateV1::Cancelled => Self::Cancelled,
        }
    }
}
