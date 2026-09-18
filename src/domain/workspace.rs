use std::collections::BTreeMap;

use super::{
    Agent, AgentId, Content, DomainCommand, DomainError, DomainEvent, EntityRef, Handoff,
    HandoffId, HandoffPayload, Name, Node, NodeId, Role, RoleId, Task, TaskId, TaskState,
    TimelineEvent, WorkspaceDirectory, WorkspaceIcon, WorkspaceId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSettings {
    name: Name,
    icon: Option<WorkspaceIcon>,
    working_directory: Option<WorkspaceDirectory>,
    instructions: Option<Content>,
}

impl WorkspaceSettings {
    pub const fn new(
        name: Name,
        icon: Option<WorkspaceIcon>,
        working_directory: Option<WorkspaceDirectory>,
        instructions: Option<Content>,
    ) -> Self {
        Self {
            name,
            icon,
            working_directory,
            instructions,
        }
    }

    pub const fn name(&self) -> &Name {
        &self.name
    }

    pub const fn icon(&self) -> Option<&WorkspaceIcon> {
        self.icon.as_ref()
    }

    pub const fn working_directory(&self) -> Option<&WorkspaceDirectory> {
        self.working_directory.as_ref()
    }

    pub const fn instructions(&self) -> Option<&Content> {
        self.instructions.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    id: WorkspaceId,
    settings: WorkspaceSettings,
    roles: BTreeMap<RoleId, Role>,
    agents: BTreeMap<AgentId, Agent>,
    tasks: BTreeMap<TaskId, Task>,
    handoffs: BTreeMap<HandoffId, Handoff>,
    nodes: BTreeMap<NodeId, Node>,
}

impl Workspace {
    pub fn new(id: WorkspaceId, name: Name) -> Self {
        Self {
            id,
            settings: WorkspaceSettings::new(name, None, None, None),
            roles: BTreeMap::new(),
            agents: BTreeMap::new(),
            tasks: BTreeMap::new(),
            handoffs: BTreeMap::new(),
            nodes: BTreeMap::new(),
        }
    }

    pub const fn id(&self) -> WorkspaceId {
        self.id
    }

    pub fn name(&self) -> &str {
        self.settings.name().as_str()
    }

    pub const fn settings(&self) -> &WorkspaceSettings {
        &self.settings
    }

    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    pub(crate) fn roles(&self) -> impl Iterator<Item = &Role> {
        self.roles.values()
    }

    pub(crate) fn agents(&self) -> impl Iterator<Item = &Agent> {
        self.agents.values()
    }

    pub(crate) fn tasks(&self) -> impl Iterator<Item = &Task> {
        self.tasks.values()
    }

    pub(crate) fn handoffs(&self) -> impl Iterator<Item = &Handoff> {
        self.handoffs.values()
    }

    pub(crate) fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    pub fn role(&self, id: RoleId) -> Option<&Role> {
        self.roles.get(&id)
    }

    pub fn agent(&self, id: AgentId) -> Option<&Agent> {
        self.agents.get(&id)
    }

    pub fn task(&self, id: TaskId) -> Option<&Task> {
        self.tasks.get(&id)
    }

    pub fn handoff(&self, id: HandoffId) -> Option<&Handoff> {
        self.handoffs.get(&id)
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn execute(&mut self, command: DomainCommand) -> Result<DomainEvent, DomainError> {
        let event = match command {
            DomainCommand::UpdateWorkspaceSettings(settings) => {
                if settings == self.settings {
                    return Err(DomainError::UnchangedWorkspaceSettings);
                }
                DomainEvent::WorkspaceSettingsChanged {
                    from: self.settings.clone(),
                    to: settings,
                }
            }
            DomainCommand::AddRole(role) => DomainEvent::RoleAdded(role),
            DomainCommand::AddAgent(agent) => DomainEvent::AgentAdded(agent),
            DomainCommand::AddTask(task) => DomainEvent::TaskAdded(task),
            DomainCommand::AddHandoff(handoff) => DomainEvent::HandoffAdded(handoff),
            DomainCommand::AddNode(node) => DomainEvent::NodeAdded(node),
            DomainCommand::TransitionAgent { agent_id, to } => {
                let agent = self
                    .agents
                    .get(&agent_id)
                    .ok_or(DomainError::EntityNotFound(EntityRef::Agent(agent_id)))?;
                DomainEvent::AgentStateChanged {
                    agent_id,
                    from: agent.state(),
                    to,
                }
            }
            DomainCommand::TransitionTask { task_id, to } => {
                let task = self
                    .tasks
                    .get(&task_id)
                    .ok_or(DomainError::EntityNotFound(EntityRef::Task(task_id)))?;
                DomainEvent::TaskStateChanged {
                    task_id,
                    from: task.state(),
                    to,
                }
            }
        };

        self.apply(&event)?;
        Ok(event)
    }

    pub fn replay(&mut self, timeline_event: &TimelineEvent) -> Result<(), DomainError> {
        if timeline_event.workspace_id() != self.id {
            return Err(DomainError::WorkspaceMismatch {
                event_id: timeline_event.id(),
                expected: self.id,
                actual: timeline_event.workspace_id(),
            });
        }

        self.apply(timeline_event.event())
    }

    pub fn apply(&mut self, event: &DomainEvent) -> Result<(), DomainError> {
        match event {
            DomainEvent::WorkspaceSettingsChanged { from, to } => {
                if &self.settings != from {
                    return Err(DomainError::WorkspaceSettingsConflict);
                }
                self.settings = to.clone();
            }
            DomainEvent::RoleAdded(role) => {
                self.ensure_absent(EntityRef::Role(role.id()))?;
                self.roles.insert(role.id(), role.clone());
            }
            DomainEvent::AgentAdded(agent) => {
                self.ensure_absent(EntityRef::Agent(agent.id()))?;
                if let Some(role_id) = agent.role_id() {
                    self.ensure_reference(
                        EntityRef::Agent(agent.id()),
                        "role_id",
                        EntityRef::Role(role_id),
                    )?;
                }
                self.agents.insert(agent.id(), agent.clone());
            }
            DomainEvent::TaskAdded(task) => {
                self.ensure_absent(EntityRef::Task(task.id()))?;
                if let Some(assignee) = task.assignee() {
                    self.ensure_reference(
                        EntityRef::Task(task.id()),
                        "assignee",
                        EntityRef::Agent(assignee),
                    )?;
                }
                if let Some(retry_of) = task.retry_of() {
                    let previous =
                        self.tasks
                            .get(&retry_of)
                            .ok_or(DomainError::InvalidReference {
                                entity: EntityRef::Task(task.id()),
                                field: "retry_of",
                                target: EntityRef::Task(retry_of),
                            })?;
                    if !matches!(previous.state(), TaskState::Failed | TaskState::Cancelled) {
                        return Err(DomainError::InvalidRetrySource {
                            task_id: task.id(),
                            retry_of,
                            state: previous.state(),
                        });
                    }
                }
                self.tasks.insert(task.id(), task.clone());
            }
            DomainEvent::HandoffAdded(handoff) => {
                self.ensure_absent(EntityRef::Handoff(handoff.id()))?;
                if handoff.source() == handoff.recipient() {
                    return Err(DomainError::SameHandoffParticipant {
                        handoff_id: handoff.id(),
                        agent_id: handoff.source(),
                    });
                }
                self.ensure_reference(
                    EntityRef::Handoff(handoff.id()),
                    "source",
                    EntityRef::Agent(handoff.source()),
                )?;
                self.ensure_reference(
                    EntityRef::Handoff(handoff.id()),
                    "recipient",
                    EntityRef::Agent(handoff.recipient()),
                )?;
                if let HandoffPayload::Task(task_id) = handoff.payload() {
                    self.ensure_reference(
                        EntityRef::Handoff(handoff.id()),
                        "payload",
                        EntityRef::Task(*task_id),
                    )?;
                }
                self.handoffs.insert(handoff.id(), handoff.clone());
            }
            DomainEvent::NodeAdded(node) => {
                self.ensure_absent(EntityRef::Node(node.id()))?;
                self.ensure_reference(EntityRef::Node(node.id()), "target", node.target().into())?;
                self.nodes.insert(node.id(), node.clone());
            }
            DomainEvent::AgentStateChanged { agent_id, from, to } => {
                let agent = self
                    .agents
                    .get(agent_id)
                    .ok_or(DomainError::EntityNotFound(EntityRef::Agent(*agent_id)))?;
                if agent.state() != *from {
                    return Err(DomainError::AgentStateConflict {
                        agent_id: *agent_id,
                        expected: *from,
                        actual: agent.state(),
                    });
                }
                if !from.can_transition_to(*to) {
                    return Err(DomainError::InvalidAgentTransition {
                        agent_id: *agent_id,
                        from: *from,
                        to: *to,
                    });
                }
                self.agents
                    .get_mut(agent_id)
                    .expect("agent existence was checked before mutation")
                    .set_state(*to);
            }
            DomainEvent::TaskStateChanged { task_id, from, to } => {
                let task = self
                    .tasks
                    .get(task_id)
                    .ok_or(DomainError::EntityNotFound(EntityRef::Task(*task_id)))?;
                if task.state() != *from {
                    return Err(DomainError::TaskStateConflict {
                        task_id: *task_id,
                        expected: *from,
                        actual: task.state(),
                    });
                }
                if !from.can_transition_to(*to) {
                    return Err(DomainError::InvalidTaskTransition {
                        task_id: *task_id,
                        from: *from,
                        to: *to,
                    });
                }
                self.tasks
                    .get_mut(task_id)
                    .expect("task existence was checked before mutation")
                    .set_state(*to);
            }
        }

        Ok(())
    }

    #[cfg(test)]
    pub(super) fn insert_agent_for_test(&mut self, agent: Agent) {
        self.agents.insert(agent.id(), agent);
    }

    #[cfg(test)]
    pub(super) fn insert_task_for_test(&mut self, task: Task) {
        self.tasks.insert(task.id(), task);
    }

    fn ensure_absent(&self, entity: EntityRef) -> Result<(), DomainError> {
        if self.contains(entity) {
            Err(DomainError::DuplicateEntity(entity))
        } else {
            Ok(())
        }
    }

    fn ensure_reference(
        &self,
        entity: EntityRef,
        field: &'static str,
        target: EntityRef,
    ) -> Result<(), DomainError> {
        if self.contains(target) {
            Ok(())
        } else {
            Err(DomainError::InvalidReference {
                entity,
                field,
                target,
            })
        }
    }

    fn contains(&self, entity: EntityRef) -> bool {
        match entity {
            EntityRef::Workspace(id) => self.id == id,
            EntityRef::Role(id) => self.roles.contains_key(&id),
            EntityRef::Agent(id) => self.agents.contains_key(&id),
            EntityRef::Task(id) => self.tasks.contains_key(&id),
            EntityRef::Handoff(id) => self.handoffs.contains_key(&id),
            EntityRef::Node(id) => self.nodes.contains_key(&id),
            EntityRef::TimelineEvent(_) => false,
        }
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new(
            WorkspaceId::new(0),
            Name::new("Welcome").expect("the default workspace name is valid"),
        )
    }
}
