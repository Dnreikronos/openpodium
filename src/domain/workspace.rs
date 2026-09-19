use std::collections::BTreeMap;

use super::{
    Agent, AgentId, CanvasLayout, ChatAttachment, ChatAttachmentId, ChatAuthor, ChatDraft,
    ChatMessage, ChatThread, ChatThreadId, CommandPreset, CommandPresetId, Connection,
    ConnectionId, ConnectionKind, Content, DomainCommand, DomainError, DomainEvent, EntityRef,
    EnvironmentProfile, EnvironmentProfileId, Handoff, HandoffId, HandoffPayload, Name, Node,
    NodeGroup, NodeGroupId, NodeId, NodeTarget, Role, RoleId, Task, TaskId, TaskState,
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
    environment_profiles: BTreeMap<EnvironmentProfileId, EnvironmentProfile>,
    command_presets: BTreeMap<CommandPresetId, CommandPreset>,
    roles: BTreeMap<RoleId, Role>,
    agents: BTreeMap<AgentId, Agent>,
    chat_threads: BTreeMap<ChatThreadId, ChatThread>,
    chat_attachments: BTreeMap<ChatAttachmentId, ChatAttachment>,
    tasks: BTreeMap<TaskId, Task>,
    handoffs: BTreeMap<HandoffId, Handoff>,
    nodes: BTreeMap<NodeId, Node>,
    groups: BTreeMap<NodeGroupId, NodeGroup>,
    connections: BTreeMap<ConnectionId, Connection>,
}

impl Workspace {
    pub fn new(id: WorkspaceId, name: Name) -> Self {
        Self {
            id,
            settings: WorkspaceSettings::new(name, None, None, None),
            environment_profiles: BTreeMap::new(),
            command_presets: BTreeMap::new(),
            roles: BTreeMap::new(),
            agents: BTreeMap::new(),
            chat_threads: BTreeMap::new(),
            chat_attachments: BTreeMap::new(),
            tasks: BTreeMap::new(),
            handoffs: BTreeMap::new(),
            nodes: BTreeMap::new(),
            groups: BTreeMap::new(),
            connections: BTreeMap::new(),
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

    pub fn environment_profiles(&self) -> impl Iterator<Item = &EnvironmentProfile> {
        self.environment_profiles.values()
    }

    pub fn environment_profile(&self, id: EnvironmentProfileId) -> Option<&EnvironmentProfile> {
        self.environment_profiles.get(&id)
    }

    pub fn command_presets(&self) -> impl Iterator<Item = &CommandPreset> {
        self.command_presets.values()
    }

    pub fn command_preset(&self, id: CommandPresetId) -> Option<&CommandPreset> {
        self.command_presets.get(&id)
    }

    pub fn roles(&self) -> impl Iterator<Item = &Role> {
        self.roles.values()
    }

    pub(crate) fn agents(&self) -> impl Iterator<Item = &Agent> {
        self.agents.values()
    }

    pub fn chat_threads(&self) -> impl Iterator<Item = &ChatThread> {
        self.chat_threads.values()
    }

    pub fn chat_attachments(&self) -> impl Iterator<Item = &ChatAttachment> {
        self.chat_attachments.values()
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

    pub(crate) fn groups(&self) -> impl Iterator<Item = &NodeGroup> {
        self.groups.values()
    }

    pub(crate) fn connections(&self) -> impl Iterator<Item = &Connection> {
        self.connections.values()
    }

    pub fn canvas_layout(&self) -> CanvasLayout {
        CanvasLayout::new(
            self.nodes.values().cloned().collect(),
            self.groups.values().cloned().collect(),
            self.connections.values().cloned().collect(),
        )
    }

    pub fn role(&self, id: RoleId) -> Option<&Role> {
        self.roles.get(&id)
    }

    pub fn agent(&self, id: AgentId) -> Option<&Agent> {
        self.agents.get(&id)
    }

    pub fn chat_thread(&self, id: ChatThreadId) -> Option<&ChatThread> {
        self.chat_threads.get(&id)
    }

    pub fn chat_attachment(&self, id: ChatAttachmentId) -> Option<&ChatAttachment> {
        self.chat_attachments.get(&id)
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
        let event =
            match command {
                DomainCommand::UpdateWorkspaceSettings(settings) => {
                    if settings == self.settings {
                        return Err(DomainError::UnchangedWorkspaceSettings);
                    }
                    DomainEvent::WorkspaceSettingsChanged {
                        from: self.settings.clone(),
                        to: settings,
                    }
                }
                DomainCommand::AddEnvironmentProfile(profile) => {
                    DomainEvent::EnvironmentProfileAdded(profile)
                }
                DomainCommand::UpdateEnvironmentProfile(profile) => {
                    let current = self.environment_profiles.get(&profile.id()).ok_or(
                        DomainError::EntityNotFound(EntityRef::EnvironmentProfile(profile.id())),
                    )?;
                    if current == &profile {
                        return Err(DomainError::UnchangedEnvironmentProfile);
                    }
                    DomainEvent::EnvironmentProfileChanged {
                        from: current.clone(),
                        to: profile,
                    }
                }
                DomainCommand::RemoveEnvironmentProfile(profile_id) => {
                    let profile = self.environment_profiles.get(&profile_id).ok_or(
                        DomainError::EntityNotFound(EntityRef::EnvironmentProfile(profile_id)),
                    )?;
                    DomainEvent::EnvironmentProfileRemoved(profile.clone())
                }
                DomainCommand::AddCommandPreset(preset) => DomainEvent::CommandPresetAdded(preset),
                DomainCommand::UpdateCommandPreset(preset) => {
                    let current = self.command_presets.get(&preset.id()).ok_or(
                        DomainError::EntityNotFound(EntityRef::CommandPreset(preset.id())),
                    )?;
                    if current == &preset {
                        return Err(DomainError::UnchangedCommandPreset);
                    }
                    DomainEvent::CommandPresetChanged {
                        from: current.clone(),
                        to: preset,
                    }
                }
                DomainCommand::RemoveCommandPreset(preset_id) => {
                    let preset =
                        self.command_presets
                            .get(&preset_id)
                            .ok_or(DomainError::EntityNotFound(EntityRef::CommandPreset(
                                preset_id,
                            )))?;
                    DomainEvent::CommandPresetRemoved(preset.clone())
                }
                DomainCommand::AddRole(role) => DomainEvent::RoleAdded(role),
                DomainCommand::UpdateRole(role) => {
                    let current = self
                        .roles
                        .get(&role.id())
                        .ok_or(DomainError::EntityNotFound(EntityRef::Role(role.id())))?;
                    if current == &role {
                        return Err(DomainError::UnchangedRole);
                    }
                    DomainEvent::RoleChanged {
                        from: current.clone(),
                        to: role,
                    }
                }
                DomainCommand::RemoveRole(role_id) => {
                    let role = self
                        .roles
                        .get(&role_id)
                        .ok_or(DomainError::EntityNotFound(EntityRef::Role(role_id)))?;
                    DomainEvent::RoleRemoved(role.clone())
                }
                DomainCommand::AssignAgentRole { agent_id, role_id } => {
                    let agent = self
                        .agents
                        .get(&agent_id)
                        .ok_or(DomainError::EntityNotFound(EntityRef::Agent(agent_id)))?;
                    if agent.role_id() == role_id {
                        return Err(DomainError::UnchangedAgentRole);
                    }
                    DomainEvent::AgentRoleChanged {
                        agent_id,
                        from: agent.role_id(),
                        to: role_id,
                    }
                }
                DomainCommand::AddAgent(agent) => DomainEvent::AgentAdded(agent),
                DomainCommand::AddChatThread(thread) => DomainEvent::ChatThreadAdded(thread),
                DomainCommand::UpdateChatThread {
                    thread_id,
                    name,
                    color,
                } => {
                    let current =
                        self.chat_threads
                            .get(&thread_id)
                            .ok_or(DomainError::EntityNotFound(EntityRef::ChatThread(
                                thread_id,
                            )))?;
                    if current.name() == &name && current.color() == &color {
                        return Err(DomainError::UnchangedChatThread);
                    }
                    DomainEvent::ChatThreadChanged {
                        thread_id,
                        from_name: current.name().clone(),
                        to_name: name,
                        from_color: current.color().clone(),
                        to_color: color,
                    }
                }
                DomainCommand::AddChatAttachment(attachment) => {
                    DomainEvent::ChatAttachmentAdded(attachment)
                }
                DomainCommand::UpdateChatDraft { thread_id, draft } => {
                    let current =
                        self.chat_threads
                            .get(&thread_id)
                            .ok_or(DomainError::EntityNotFound(EntityRef::ChatThread(
                                thread_id,
                            )))?;
                    if current.draft() == &draft {
                        return Err(DomainError::UnchangedChatDraft);
                    }
                    DomainEvent::ChatDraftChanged {
                        thread_id,
                        from: current.draft().clone(),
                        to: draft,
                    }
                }
                DomainCommand::SubmitChatDraft {
                    thread_id,
                    message_id,
                    sent_at,
                } => {
                    let thread =
                        self.chat_threads
                            .get(&thread_id)
                            .ok_or(DomainError::EntityNotFound(EntityRef::ChatThread(
                                thread_id,
                            )))?;
                    DomainEvent::ChatDraftSubmitted {
                        thread_id,
                        from: thread.draft().clone(),
                        message: ChatMessage::from_draft(
                            message_id,
                            thread_id,
                            thread.draft(),
                            sent_at,
                        )?,
                    }
                }
                DomainCommand::AppendAgentChatMessage(message) => {
                    DomainEvent::AgentChatMessageAppended(message)
                }
                DomainCommand::AddTask(task) => DomainEvent::TaskAdded(task),
                DomainCommand::AddHandoff(handoff) => DomainEvent::HandoffAdded(handoff),
                DomainCommand::AddNode(node) => DomainEvent::NodeAdded(node),
                DomainCommand::AddAgentNode { agent, node } => {
                    DomainEvent::AgentNodeAdded { agent, node }
                }
                DomainCommand::ReplaceCanvas { before, after } => {
                    DomainEvent::CanvasReplaced { before, after }
                }
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
            DomainEvent::EnvironmentProfileAdded(profile) => {
                self.ensure_absent(EntityRef::EnvironmentProfile(profile.id()))?;
                self.environment_profiles
                    .insert(profile.id(), profile.clone());
            }
            DomainEvent::EnvironmentProfileChanged { from, to } => {
                let current = self.environment_profiles.get(&from.id()).ok_or(
                    DomainError::EntityNotFound(EntityRef::EnvironmentProfile(from.id())),
                )?;
                if current != from || to.id() != from.id() {
                    return Err(DomainError::EnvironmentProfileConflict(from.id()));
                }
                self.environment_profiles.insert(to.id(), to.clone());
            }
            DomainEvent::EnvironmentProfileRemoved(profile) => {
                let current = self.environment_profiles.get(&profile.id()).ok_or(
                    DomainError::EntityNotFound(EntityRef::EnvironmentProfile(profile.id())),
                )?;
                if current != profile {
                    return Err(DomainError::EnvironmentProfileConflict(profile.id()));
                }
                if let Some(agent) = self
                    .agents
                    .values()
                    .find(|agent| agent.environment_id() == Some(profile.id()))
                {
                    return Err(DomainError::EnvironmentProfileInUse {
                        profile_id: profile.id(),
                        agent_id: agent.id(),
                    });
                }
                self.environment_profiles.remove(&profile.id());
            }
            DomainEvent::CommandPresetAdded(preset) => {
                self.ensure_absent(EntityRef::CommandPreset(preset.id()))?;
                self.command_presets.insert(preset.id(), preset.clone());
            }
            DomainEvent::CommandPresetChanged { from, to } => {
                let current =
                    self.command_presets
                        .get(&from.id())
                        .ok_or(DomainError::EntityNotFound(EntityRef::CommandPreset(
                            from.id(),
                        )))?;
                if current != from || to.id() != from.id() {
                    return Err(DomainError::CommandPresetConflict(from.id()));
                }
                self.command_presets.insert(to.id(), to.clone());
            }
            DomainEvent::CommandPresetRemoved(preset) => {
                let current =
                    self.command_presets
                        .get(&preset.id())
                        .ok_or(DomainError::EntityNotFound(EntityRef::CommandPreset(
                            preset.id(),
                        )))?;
                if current != preset {
                    return Err(DomainError::CommandPresetConflict(preset.id()));
                }
                if let Some(agent) = self
                    .agents
                    .values()
                    .find(|agent| agent.program() == super::AgentProgram::Custom(preset.id()))
                {
                    return Err(DomainError::CommandPresetInUse {
                        preset_id: preset.id(),
                        agent_id: agent.id(),
                    });
                }
                self.command_presets.remove(&preset.id());
            }
            DomainEvent::RoleAdded(role) => {
                self.ensure_absent(EntityRef::Role(role.id()))?;
                self.roles.insert(role.id(), role.clone());
            }
            DomainEvent::RoleChanged { from, to } => {
                let current = self
                    .roles
                    .get(&from.id())
                    .ok_or(DomainError::EntityNotFound(EntityRef::Role(from.id())))?;
                if current != from || to.id() != from.id() {
                    return Err(DomainError::RoleConflict(from.id()));
                }
                self.roles.insert(to.id(), to.clone());
            }
            DomainEvent::RoleRemoved(role) => {
                let current = self
                    .roles
                    .get(&role.id())
                    .ok_or(DomainError::EntityNotFound(EntityRef::Role(role.id())))?;
                if current != role {
                    return Err(DomainError::RoleConflict(role.id()));
                }
                if let Some(agent) = self
                    .agents
                    .values()
                    .find(|agent| agent.role_id() == Some(role.id()))
                {
                    return Err(DomainError::RoleInUse {
                        role_id: role.id(),
                        agent_id: agent.id(),
                    });
                }
                self.roles.remove(&role.id());
            }
            DomainEvent::AgentRoleChanged { agent_id, from, to } => {
                let agent = self
                    .agents
                    .get(agent_id)
                    .ok_or(DomainError::EntityNotFound(EntityRef::Agent(*agent_id)))?;
                if agent.role_id() != *from {
                    return Err(DomainError::AgentRoleConflict {
                        agent_id: *agent_id,
                        expected: *from,
                        actual: agent.role_id(),
                    });
                }
                if let Some(role_id) = to {
                    self.ensure_reference(
                        EntityRef::Agent(*agent_id),
                        "role_id",
                        EntityRef::Role(*role_id),
                    )?;
                }
                self.agents
                    .get_mut(agent_id)
                    .expect("agent existence was checked before mutation")
                    .set_role_id(*to);
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
                if let Some(environment_id) = agent.environment_id() {
                    self.ensure_reference(
                        EntityRef::Agent(agent.id()),
                        "environment_id",
                        EntityRef::EnvironmentProfile(environment_id),
                    )?;
                }
                if let super::AgentProgram::Custom(preset_id) = agent.program() {
                    self.ensure_reference(
                        EntityRef::Agent(agent.id()),
                        "program",
                        EntityRef::CommandPreset(preset_id),
                    )?;
                }
                self.agents.insert(agent.id(), agent.clone());
            }
            DomainEvent::ChatThreadAdded(thread) => {
                self.ensure_absent(EntityRef::ChatThread(thread.id()))?;
                self.ensure_reference(
                    EntityRef::ChatThread(thread.id()),
                    "agent_id",
                    EntityRef::Agent(thread.agent_id()),
                )?;
                if !thread.draft().is_empty() || !thread.messages().is_empty() {
                    return Err(DomainError::ChatThreadConflict(thread.id()));
                }
                self.chat_threads.insert(thread.id(), thread.clone());
            }
            DomainEvent::ChatThreadChanged {
                thread_id,
                from_name,
                to_name,
                from_color,
                to_color,
            } => {
                let thread =
                    self.chat_threads
                        .get(thread_id)
                        .ok_or(DomainError::EntityNotFound(EntityRef::ChatThread(
                            *thread_id,
                        )))?;
                if thread.name() != from_name || thread.color() != from_color {
                    return Err(DomainError::ChatThreadConflict(*thread_id));
                }
                self.chat_threads
                    .get_mut(thread_id)
                    .expect("thread existence was checked before mutation")
                    .set_appearance(to_name.clone(), to_color.clone());
            }
            DomainEvent::ChatAttachmentAdded(attachment) => {
                self.ensure_absent(EntityRef::ChatAttachment(attachment.id()))?;
                self.ensure_reference(
                    EntityRef::ChatAttachment(attachment.id()),
                    "thread_id",
                    EntityRef::ChatThread(attachment.thread_id()),
                )?;
                self.chat_attachments
                    .insert(attachment.id(), attachment.clone());
            }
            DomainEvent::ChatDraftChanged {
                thread_id,
                from,
                to,
            } => {
                let thread =
                    self.chat_threads
                        .get(thread_id)
                        .ok_or(DomainError::EntityNotFound(EntityRef::ChatThread(
                            *thread_id,
                        )))?;
                if thread.draft() != from {
                    return Err(DomainError::ChatDraftConflict(*thread_id));
                }
                self.validate_chat_references(*thread_id, to.attachments(), to.mentions())?;
                self.chat_threads
                    .get_mut(thread_id)
                    .expect("thread existence was checked before mutation")
                    .set_draft(to.clone());
            }
            DomainEvent::ChatDraftSubmitted {
                thread_id,
                from,
                message,
            } => {
                let thread =
                    self.chat_threads
                        .get(thread_id)
                        .ok_or(DomainError::EntityNotFound(EntityRef::ChatThread(
                            *thread_id,
                        )))?;
                if thread.draft() != from {
                    return Err(DomainError::ChatDraftConflict(*thread_id));
                }
                let expected =
                    ChatMessage::from_draft(message.id(), *thread_id, from, message.sent_at())?;
                if &expected != message {
                    return Err(DomainError::ChatMessageConflict(message.id()));
                }
                self.ensure_absent(EntityRef::ChatMessage(message.id()))?;
                self.validate_chat_references(
                    *thread_id,
                    message.attachments(),
                    message.mentions(),
                )?;
                let thread = self
                    .chat_threads
                    .get_mut(thread_id)
                    .expect("thread existence was checked before mutation");
                thread.push_message(message.clone());
                thread.set_draft(ChatDraft::default());
            }
            DomainEvent::AgentChatMessageAppended(message) => {
                let thread = self.chat_threads.get(&message.thread_id()).ok_or(
                    DomainError::EntityNotFound(EntityRef::ChatThread(message.thread_id())),
                )?;
                if message.author() != ChatAuthor::Agent(thread.agent_id()) {
                    return Err(DomainError::InvalidChatAuthor {
                        message_id: message.id(),
                        expected: thread.agent_id(),
                    });
                }
                self.ensure_absent(EntityRef::ChatMessage(message.id()))?;
                self.validate_chat_references(
                    message.thread_id(),
                    message.attachments(),
                    message.mentions(),
                )?;
                self.chat_threads
                    .get_mut(&message.thread_id())
                    .expect("thread existence was checked before mutation")
                    .push_message(message.clone());
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
            DomainEvent::AgentNodeAdded { agent, node } => {
                self.ensure_absent(EntityRef::Agent(agent.id()))?;
                self.ensure_absent(EntityRef::Node(node.id()))?;
                if let Some(role_id) = agent.role_id() {
                    self.ensure_reference(
                        EntityRef::Agent(agent.id()),
                        "role_id",
                        EntityRef::Role(role_id),
                    )?;
                }
                if let Some(environment_id) = agent.environment_id() {
                    self.ensure_reference(
                        EntityRef::Agent(agent.id()),
                        "environment_id",
                        EntityRef::EnvironmentProfile(environment_id),
                    )?;
                }
                if let super::AgentProgram::Custom(preset_id) = agent.program() {
                    self.ensure_reference(
                        EntityRef::Agent(agent.id()),
                        "program",
                        EntityRef::CommandPreset(preset_id),
                    )?;
                }
                if node.target() != super::NodeTarget::Agent(agent.id()) {
                    return Err(DomainError::InvalidReference {
                        entity: EntityRef::Node(node.id()),
                        field: "target",
                        target: node.target().into(),
                    });
                }
                self.agents.insert(agent.id(), agent.clone());
                self.nodes.insert(node.id(), node.clone());
            }
            DomainEvent::CanvasReplaced { before, after } => {
                if &self.canvas_layout() != before {
                    return Err(DomainError::CanvasConflict);
                }
                self.validate_canvas(after)?;
                self.nodes = after
                    .nodes()
                    .iter()
                    .cloned()
                    .map(|node| (node.id(), node))
                    .collect();
                self.groups = after
                    .groups()
                    .iter()
                    .cloned()
                    .map(|group| (group.id(), group))
                    .collect();
                self.connections = after
                    .connections()
                    .iter()
                    .cloned()
                    .map(|connection| (connection.id(), connection))
                    .collect();
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
            EntityRef::EnvironmentProfile(id) => self.environment_profiles.contains_key(&id),
            EntityRef::CommandPreset(id) => self.command_presets.contains_key(&id),
            EntityRef::Role(id) => self.roles.contains_key(&id),
            EntityRef::Agent(id) => self.agents.contains_key(&id),
            EntityRef::ChatThread(id) => self.chat_threads.contains_key(&id),
            EntityRef::ChatMessage(id) => self
                .chat_threads
                .values()
                .any(|thread| thread.messages().iter().any(|message| message.id() == id)),
            EntityRef::ChatAttachment(id) => self.chat_attachments.contains_key(&id),
            EntityRef::Task(id) => self.tasks.contains_key(&id),
            EntityRef::Handoff(id) => self.handoffs.contains_key(&id),
            EntityRef::Node(id) => self.nodes.contains_key(&id),
            EntityRef::NodeGroup(id) => self.groups.contains_key(&id),
            EntityRef::Connection(id) => self.connections.contains_key(&id),
            EntityRef::TimelineEvent(_) => false,
        }
    }

    fn validate_chat_references(
        &self,
        thread_id: ChatThreadId,
        attachments: &[ChatAttachmentId],
        mentions: &[NodeTarget],
    ) -> Result<(), DomainError> {
        let thread = self
            .chat_threads
            .get(&thread_id)
            .ok_or(DomainError::EntityNotFound(EntityRef::ChatThread(
                thread_id,
            )))?;
        let mut total_bytes = 0_u64;
        for attachment_id in attachments {
            let attachment =
                self.chat_attachments
                    .get(attachment_id)
                    .ok_or(DomainError::EntityNotFound(EntityRef::ChatAttachment(
                        *attachment_id,
                    )))?;
            if attachment.thread_id() != thread_id {
                return Err(DomainError::ChatAttachmentWrongThread {
                    attachment_id: *attachment_id,
                    expected: thread_id,
                    actual: attachment.thread_id(),
                });
            }
            total_bytes = total_bytes.saturating_add(attachment.byte_len());
        }
        if total_bytes > ChatDraft::MAX_TOTAL_ATTACHMENT_BYTES {
            return Err(DomainError::ChatAttachmentTotalTooLarge {
                thread_id,
                max_bytes: ChatDraft::MAX_TOTAL_ATTACHMENT_BYTES,
                actual_bytes: total_bytes,
            });
        }

        let owner_nodes: Vec<NodeId> = self
            .nodes
            .values()
            .filter(|node| node.target() == NodeTarget::Agent(thread.agent_id()))
            .map(Node::id)
            .collect();
        for mention in mentions {
            self.ensure_reference(
                EntityRef::ChatThread(thread_id),
                "mention",
                (*mention).into(),
            )?;
            let target_nodes: Vec<NodeId> = self
                .nodes
                .values()
                .filter(|node| node.target() == *mention)
                .map(Node::id)
                .collect();
            let connected = self.connections.values().any(|connection| {
                (owner_nodes.contains(&connection.source())
                    && target_nodes.contains(&connection.target()))
                    || (owner_nodes.contains(&connection.target())
                        && target_nodes.contains(&connection.source()))
            });
            if !connected {
                return Err(DomainError::MentionNotConnected {
                    thread_id,
                    target: *mention,
                });
            }
        }
        Ok(())
    }

    fn validate_canvas(&self, layout: &CanvasLayout) -> Result<(), DomainError> {
        let mut node_ids = std::collections::BTreeSet::new();
        for node in layout.nodes() {
            if !node_ids.insert(node.id()) {
                return Err(DomainError::DuplicateEntity(EntityRef::Node(node.id())));
            }
            self.ensure_reference(EntityRef::Node(node.id()), "target", node.target().into())?;
        }

        let mut grouped_nodes = std::collections::BTreeSet::new();
        let mut group_ids = std::collections::BTreeSet::new();
        for group in layout.groups() {
            if !group_ids.insert(group.id()) {
                return Err(DomainError::DuplicateEntity(EntityRef::NodeGroup(
                    group.id(),
                )));
            }
            if group.len() < 2 {
                return Err(DomainError::InvalidGroup {
                    group_id: group.id(),
                    detail: "a group must contain at least two nodes",
                });
            }
            for node_id in group.members() {
                if !node_ids.contains(&node_id) {
                    return Err(DomainError::InvalidGroup {
                        group_id: group.id(),
                        detail: "a member node does not exist",
                    });
                }
                if !grouped_nodes.insert(node_id) {
                    return Err(DomainError::NodeInMultipleGroups { node_id });
                }
            }
        }

        let nodes = layout
            .nodes()
            .iter()
            .map(|node| (node.id(), node))
            .collect::<BTreeMap<_, _>>();
        let mut connection_ids = std::collections::BTreeSet::new();
        let mut endpoints = std::collections::BTreeSet::new();
        for connection in layout.connections() {
            if !connection_ids.insert(connection.id()) {
                return Err(DomainError::DuplicateEntity(EntityRef::Connection(
                    connection.id(),
                )));
            }
            if connection.source() == connection.target() {
                return Err(DomainError::InvalidConnection {
                    connection_id: connection.id(),
                    detail: "source and target must differ",
                });
            }
            let Some(source) = nodes.get(&connection.source()) else {
                return Err(DomainError::InvalidConnection {
                    connection_id: connection.id(),
                    detail: "source node does not exist",
                });
            };
            let Some(target) = nodes.get(&connection.target()) else {
                return Err(DomainError::InvalidConnection {
                    connection_id: connection.id(),
                    detail: "target node does not exist",
                });
            };
            if ConnectionKind::between(source.target(), target.target()) != Some(connection.kind())
            {
                return Err(DomainError::InvalidConnection {
                    connection_id: connection.id(),
                    detail: "kind is incompatible with its endpoint targets",
                });
            }
            if !endpoints.insert((connection.source(), connection.target())) {
                return Err(DomainError::DuplicateConnection {
                    source: connection.source(),
                    target: connection.target(),
                });
            }
        }

        Ok(())
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
