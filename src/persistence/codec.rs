use serde::{Deserialize, Serialize};

use crate::domain::{
    Agent, AgentId, AgentProgram, AgentState, CanvasLayout, CanvasPoint, CanvasSize,
    ChatAttachment, ChatAttachmentId, ChatAuthor, ChatDraft, ChatMessage, ChatMessageId,
    ChatThread, ChatThreadId, CommandPreset, CommandPresetId, Connection, ConnectionId,
    ConnectionKind, ContainerEnvironment, Content, CustomEnvironment, DeliveryAttempt,
    DeliveryMechanism, DeliveryOutcome, DomainCommand, DomainEvent, EnvironmentKind,
    EnvironmentProfile, EnvironmentProfileId, Handoff, HandoffId, HandoffMessageId, HandoffPayload,
    HandoffProgress, HandoffResponse, HandoffResponseStatus, HandoffTermination, Name, Node,
    NodeGroup, NodeGroupId, NodeId, NodeTarget, Role, RoleColor, RoleIcon, RoleId, SshEnvironment,
    Task, TaskId, TaskState, ThreadColor, Timestamp, Workspace, WorkspaceDirectory, WorkspaceIcon,
    WorkspaceId, WorkspaceSettings,
};

use super::PersistenceError;

pub(crate) const EVENT_FORMAT_VERSION: u32 = 8;
pub(crate) const SNAPSHOT_FORMAT_VERSION: u32 = 7;

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
    if format_version < 4 && stored.requires_version_four() {
        return Err(PersistenceError::invalid_record(
            "domain event",
            sequence,
            "environment profiles and agent environment references require event format version 4",
        ));
    }
    if format_version < 5 && stored.requires_version_five() {
        return Err(PersistenceError::invalid_record(
            "domain event",
            sequence,
            "command presets, role appearance, and role changes require event format version 5",
        ));
    }
    if format_version < 6 && stored.requires_version_six() {
        return Err(PersistenceError::invalid_record(
            "domain event",
            sequence,
            "chat threads, messages, drafts, and attachments require event format version 6",
        ));
    }
    if format_version < 7 && stored.requires_version_seven() {
        return Err(PersistenceError::invalid_record(
            "domain event",
            sequence,
            "typed handoffs and orchestration history require event format version 7",
        ));
    }
    if format_version < 8 && stored.requires_version_eight() {
        return Err(PersistenceError::invalid_record(
            "domain event",
            sequence,
            "atomic task recovery requires event format version 8",
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
    EnvironmentProfileAdded {
        profile: EnvironmentProfileV1,
    },
    EnvironmentProfileChanged {
        from: EnvironmentProfileV1,
        to: EnvironmentProfileV1,
    },
    EnvironmentProfileRemoved {
        profile: EnvironmentProfileV1,
    },
    CommandPresetAdded {
        preset: CommandPresetV1,
    },
    CommandPresetChanged {
        from: CommandPresetV1,
        to: CommandPresetV1,
    },
    CommandPresetRemoved {
        preset: CommandPresetV1,
    },
    RoleAdded {
        role: RoleV1,
    },
    RoleChanged {
        from: RoleV1,
        to: RoleV1,
    },
    RoleRemoved {
        role: RoleV1,
    },
    AgentRoleChanged {
        agent_id: u64,
        from: Option<u64>,
        to: Option<u64>,
    },
    AgentAdded {
        agent: AgentV1,
    },
    ChatThreadAdded {
        thread: ChatThreadV1,
    },
    ChatThreadChanged {
        thread_id: u64,
        from_name: String,
        to_name: String,
        from_color: String,
        to_color: String,
    },
    ChatAttachmentAdded {
        attachment: ChatAttachmentV1,
    },
    ChatDraftChanged {
        thread_id: u64,
        from: ChatDraftV1,
        to: ChatDraftV1,
    },
    ChatDraftSubmitted {
        thread_id: u64,
        from: ChatDraftV1,
        message: ChatMessageV1,
    },
    AgentChatMessageAppended {
        message: ChatMessageV1,
    },
    TaskAdded {
        task: TaskV1,
    },
    HandoffAdded {
        handoff: HandoffV1,
    },
    TaskHandoffAdded {
        task: TaskV1,
        handoff: HandoffV1,
    },
    HandoffChanged {
        before: HandoffV1,
        after: HandoffV1,
    },
    TaskCancelled {
        task_id: u64,
        from: TaskStateV1,
        before: HandoffV1,
        after: HandoffV1,
    },
    TaskResumed {
        task_id: u64,
        from: TaskStateV1,
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
            DomainEvent::EnvironmentProfileAdded(profile) => Self::EnvironmentProfileAdded {
                profile: EnvironmentProfileV1::from(profile),
            },
            DomainEvent::EnvironmentProfileChanged { from, to } => {
                Self::EnvironmentProfileChanged {
                    from: EnvironmentProfileV1::from(from),
                    to: EnvironmentProfileV1::from(to),
                }
            }
            DomainEvent::EnvironmentProfileRemoved(profile) => Self::EnvironmentProfileRemoved {
                profile: EnvironmentProfileV1::from(profile),
            },
            DomainEvent::CommandPresetAdded(preset) => Self::CommandPresetAdded {
                preset: CommandPresetV1::from(preset),
            },
            DomainEvent::CommandPresetChanged { from, to } => Self::CommandPresetChanged {
                from: CommandPresetV1::from(from),
                to: CommandPresetV1::from(to),
            },
            DomainEvent::CommandPresetRemoved(preset) => Self::CommandPresetRemoved {
                preset: CommandPresetV1::from(preset),
            },
            DomainEvent::RoleAdded(role) => Self::RoleAdded {
                role: RoleV1::from(role),
            },
            DomainEvent::RoleChanged { from, to } => Self::RoleChanged {
                from: RoleV1::from(from),
                to: RoleV1::from(to),
            },
            DomainEvent::RoleRemoved(role) => Self::RoleRemoved {
                role: RoleV1::from(role),
            },
            DomainEvent::AgentRoleChanged { agent_id, from, to } => Self::AgentRoleChanged {
                agent_id: agent_id.get(),
                from: from.map(RoleId::get),
                to: to.map(RoleId::get),
            },
            DomainEvent::AgentAdded(agent) => Self::AgentAdded {
                agent: AgentV1::from(agent),
            },
            DomainEvent::ChatThreadAdded(thread) => Self::ChatThreadAdded {
                thread: ChatThreadV1::from(thread),
            },
            DomainEvent::ChatThreadChanged {
                thread_id,
                from_name,
                to_name,
                from_color,
                to_color,
            } => Self::ChatThreadChanged {
                thread_id: thread_id.get(),
                from_name: from_name.as_str().to_owned(),
                to_name: to_name.as_str().to_owned(),
                from_color: from_color.as_str().to_owned(),
                to_color: to_color.as_str().to_owned(),
            },
            DomainEvent::ChatAttachmentAdded(attachment) => Self::ChatAttachmentAdded {
                attachment: ChatAttachmentV1::from(attachment),
            },
            DomainEvent::ChatDraftChanged {
                thread_id,
                from,
                to,
            } => Self::ChatDraftChanged {
                thread_id: thread_id.get(),
                from: ChatDraftV1::from(from),
                to: ChatDraftV1::from(to),
            },
            DomainEvent::ChatDraftSubmitted {
                thread_id,
                from,
                message,
            } => Self::ChatDraftSubmitted {
                thread_id: thread_id.get(),
                from: ChatDraftV1::from(from),
                message: ChatMessageV1::from(message),
            },
            DomainEvent::AgentChatMessageAppended(message) => Self::AgentChatMessageAppended {
                message: ChatMessageV1::from(message),
            },
            DomainEvent::TaskAdded(task) => Self::TaskAdded {
                task: TaskV1::from(task),
            },
            DomainEvent::HandoffAdded(handoff) => Self::HandoffAdded {
                handoff: HandoffV1::from(handoff),
            },
            DomainEvent::TaskHandoffAdded { task, handoff } => Self::TaskHandoffAdded {
                task: TaskV1::from(task),
                handoff: HandoffV1::from(handoff),
            },
            DomainEvent::HandoffChanged { before, after } => Self::HandoffChanged {
                before: HandoffV1::from(before),
                after: HandoffV1::from(after),
            },
            DomainEvent::TaskCancelled {
                task_id,
                from,
                before,
                after,
            } => Self::TaskCancelled {
                task_id: task_id.get(),
                from: (*from).into(),
                before: HandoffV1::from(before),
                after: HandoffV1::from(after),
            },
            DomainEvent::TaskResumed {
                task_id,
                from,
                handoff,
            } => Self::TaskResumed {
                task_id: task_id.get(),
                from: (*from).into(),
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

    fn requires_version_four(&self) -> bool {
        match self {
            Self::EnvironmentProfileAdded { .. }
            | Self::EnvironmentProfileChanged { .. }
            | Self::EnvironmentProfileRemoved { .. } => true,
            Self::AgentAdded { agent } | Self::AgentNodeAdded { agent, .. } => {
                agent.environment_id.is_some()
            }
            _ => false,
        }
    }

    fn requires_version_five(&self) -> bool {
        match self {
            Self::CommandPresetAdded { .. }
            | Self::CommandPresetChanged { .. }
            | Self::CommandPresetRemoved { .. }
            | Self::RoleChanged { .. }
            | Self::RoleRemoved { .. }
            | Self::AgentRoleChanged { .. } => true,
            Self::RoleAdded { role } => role.has_appearance_fields(),
            Self::AgentAdded { agent } | Self::AgentNodeAdded { agent, .. } => {
                matches!(
                    agent.program,
                    AgentProgramV1::OpenCode | AgentProgramV1::Custom { .. }
                )
            }
            _ => false,
        }
    }

    fn requires_version_six(&self) -> bool {
        matches!(
            self,
            Self::ChatThreadAdded { .. }
                | Self::ChatThreadChanged { .. }
                | Self::ChatAttachmentAdded { .. }
                | Self::ChatDraftChanged { .. }
                | Self::ChatDraftSubmitted { .. }
                | Self::AgentChatMessageAppended { .. }
        )
    }

    fn requires_version_seven(&self) -> bool {
        match self {
            Self::TaskHandoffAdded { .. } | Self::HandoffChanged { .. } => true,
            Self::HandoffAdded { handoff } => handoff.has_orchestration_fields(),
            _ => false,
        }
    }

    fn requires_version_eight(&self) -> bool {
        matches!(self, Self::TaskCancelled { .. } | Self::TaskResumed { .. })
    }

    fn into_domain(self) -> Result<DomainEvent, String> {
        match self {
            Self::WorkspaceSettingsChanged { from, to } => {
                Ok(DomainEvent::WorkspaceSettingsChanged {
                    from: from.into_domain()?,
                    to: to.into_domain()?,
                })
            }
            Self::EnvironmentProfileAdded { profile } => {
                Ok(DomainEvent::EnvironmentProfileAdded(profile.into_domain()?))
            }
            Self::EnvironmentProfileChanged { from, to } => {
                Ok(DomainEvent::EnvironmentProfileChanged {
                    from: from.into_domain()?,
                    to: to.into_domain()?,
                })
            }
            Self::EnvironmentProfileRemoved { profile } => Ok(
                DomainEvent::EnvironmentProfileRemoved(profile.into_domain()?),
            ),
            Self::CommandPresetAdded { preset } => {
                Ok(DomainEvent::CommandPresetAdded(preset.into_domain()?))
            }
            Self::CommandPresetChanged { from, to } => Ok(DomainEvent::CommandPresetChanged {
                from: from.into_domain()?,
                to: to.into_domain()?,
            }),
            Self::CommandPresetRemoved { preset } => {
                Ok(DomainEvent::CommandPresetRemoved(preset.into_domain()?))
            }
            Self::RoleAdded { role } => Ok(DomainEvent::RoleAdded(role.into_domain()?)),
            Self::RoleChanged { from, to } => Ok(DomainEvent::RoleChanged {
                from: from.into_domain()?,
                to: to.into_domain()?,
            }),
            Self::RoleRemoved { role } => Ok(DomainEvent::RoleRemoved(role.into_domain()?)),
            Self::AgentRoleChanged { agent_id, from, to } => Ok(DomainEvent::AgentRoleChanged {
                agent_id: AgentId::new(agent_id),
                from: from.map(RoleId::new),
                to: to.map(RoleId::new),
            }),
            Self::AgentAdded { agent } => {
                let (agent, state) = agent.into_domain()?;
                if state != AgentState::Starting {
                    return Err("an agent_added event must contain a starting agent".to_owned());
                }
                Ok(DomainEvent::AgentAdded(agent))
            }
            Self::ChatThreadAdded { thread } => {
                Ok(DomainEvent::ChatThreadAdded(thread.into_domain()?))
            }
            Self::ChatThreadChanged {
                thread_id,
                from_name,
                to_name,
                from_color,
                to_color,
            } => Ok(DomainEvent::ChatThreadChanged {
                thread_id: ChatThreadId::new(thread_id),
                from_name: Name::new(from_name).map_err(|error| error.to_string())?,
                to_name: Name::new(to_name).map_err(|error| error.to_string())?,
                from_color: ThreadColor::new(from_color).map_err(|error| error.to_string())?,
                to_color: ThreadColor::new(to_color).map_err(|error| error.to_string())?,
            }),
            Self::ChatAttachmentAdded { attachment } => {
                Ok(DomainEvent::ChatAttachmentAdded(attachment.into_domain()?))
            }
            Self::ChatDraftChanged {
                thread_id,
                from,
                to,
            } => Ok(DomainEvent::ChatDraftChanged {
                thread_id: ChatThreadId::new(thread_id),
                from: from.into_domain()?,
                to: to.into_domain()?,
            }),
            Self::ChatDraftSubmitted {
                thread_id,
                from,
                message,
            } => Ok(DomainEvent::ChatDraftSubmitted {
                thread_id: ChatThreadId::new(thread_id),
                from: from.into_domain()?,
                message: message.into_domain()?,
            }),
            Self::AgentChatMessageAppended { message } => Ok(
                DomainEvent::AgentChatMessageAppended(message.into_domain()?),
            ),
            Self::TaskAdded { task } => {
                let (task, state) = task.into_domain()?;
                if state != TaskState::Queued {
                    return Err("a task_added event must contain a queued task".to_owned());
                }
                Ok(DomainEvent::TaskAdded(task))
            }
            Self::HandoffAdded { handoff } => Ok(DomainEvent::HandoffAdded(handoff.into_domain()?)),
            Self::TaskHandoffAdded { task, handoff } => {
                let (task, state) = task.into_domain()?;
                if state != TaskState::Queued {
                    return Err("a task_handoff_added event must contain a queued task".to_owned());
                }
                Ok(DomainEvent::TaskHandoffAdded {
                    task,
                    handoff: handoff.into_domain()?,
                })
            }
            Self::HandoffChanged { before, after } => Ok(DomainEvent::HandoffChanged {
                before: before.into_domain()?,
                after: after.into_domain()?,
            }),
            Self::TaskCancelled {
                task_id,
                from,
                before,
                after,
            } => Ok(DomainEvent::TaskCancelled {
                task_id: TaskId::new(task_id),
                from: from.into(),
                before: before.into_domain()?,
                after: after.into_domain()?,
            }),
            Self::TaskResumed {
                task_id,
                from,
                handoff,
            } => Ok(DomainEvent::TaskResumed {
                task_id: TaskId::new(task_id),
                from: from.into(),
                handoff: handoff.into_domain()?,
            }),
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
    #[serde(default)]
    environment_profiles: Vec<EnvironmentProfileV1>,
    #[serde(default)]
    command_presets: Vec<CommandPresetV1>,
    roles: Vec<RoleV1>,
    agents: Vec<AgentV1>,
    #[serde(default)]
    chat_threads: Vec<ChatThreadV1>,
    #[serde(default)]
    chat_attachments: Vec<ChatAttachmentV1>,
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
            environment_profiles: workspace
                .environment_profiles()
                .map(EnvironmentProfileV1::from)
                .collect(),
            command_presets: workspace
                .command_presets()
                .map(CommandPresetV1::from)
                .collect(),
            roles: workspace.roles().map(RoleV1::from).collect(),
            agents: workspace.agents().map(AgentV1::from).collect(),
            chat_threads: workspace.chat_threads().map(ChatThreadV1::from).collect(),
            chat_attachments: workspace
                .chat_attachments()
                .map(ChatAttachmentV1::from)
                .collect(),
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
        if format_version < 4
            && (!self.environment_profiles.is_empty()
                || self
                    .agents
                    .iter()
                    .any(|agent| agent.environment_id.is_some()))
        {
            return Err(
                "environment profiles and agent environment references require snapshot format version 4"
                    .to_owned(),
            );
        }
        if format_version < 5
            && (!self.command_presets.is_empty()
                || self.roles.iter().any(RoleV1::has_appearance_fields)
                || self.agents.iter().any(|agent| {
                    matches!(
                        agent.program,
                        AgentProgramV1::OpenCode | AgentProgramV1::Custom { .. }
                    )
                }))
        {
            return Err(
                "command presets and role appearance require snapshot format version 5".to_owned(),
            );
        }
        if format_version < 6
            && (!self.chat_threads.is_empty() || !self.chat_attachments.is_empty())
        {
            return Err(
                "chat threads and attachments require snapshot format version 6".to_owned(),
            );
        }
        if format_version < 7
            && self
                .handoffs
                .iter()
                .any(HandoffV1::has_orchestration_fields)
        {
            return Err(
                "typed handoffs and orchestration history require snapshot format version 7"
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

        for profile in self.environment_profiles {
            apply_snapshot_command(
                &mut workspace,
                DomainCommand::AddEnvironmentProfile(profile.into_domain()?),
            )?;
        }

        for preset in self.command_presets {
            apply_snapshot_command(
                &mut workspace,
                DomainCommand::AddCommandPreset(preset.into_domain()?),
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

        let mut pending_handoffs = self.handoffs;
        while !pending_handoffs.is_empty() {
            let Some(index) = pending_handoffs.iter().position(|handoff| {
                handoff
                    .parent
                    .is_none_or(|parent| workspace.handoff(HandoffId::new(parent)).is_some())
            }) else {
                return Err(
                    "handoff parent references contain a cycle or missing handoff".to_owned(),
                );
            };
            apply_snapshot_command(
                &mut workspace,
                DomainCommand::AddHandoff(pending_handoffs.remove(index).into_domain()?),
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

        let mut chat_threads = Vec::with_capacity(self.chat_threads.len());
        for stored in self.chat_threads {
            let (thread, draft, messages) = stored.into_parts()?;
            apply_snapshot_command(&mut workspace, DomainCommand::AddChatThread(thread.clone()))?;
            chat_threads.push((thread.id(), draft, messages));
        }
        for attachment in self.chat_attachments {
            apply_snapshot_command(
                &mut workspace,
                DomainCommand::AddChatAttachment(attachment.into_domain()?),
            )?;
        }
        for (thread_id, draft, messages) in chat_threads {
            for message in messages {
                match message.author() {
                    ChatAuthor::User => {
                        let message_draft = ChatDraft::new(
                            message.content().as_str(),
                            message.attachments().to_vec(),
                            message.mentions().to_vec(),
                        )
                        .map_err(|error| error.to_string())?;
                        apply_snapshot_command(
                            &mut workspace,
                            DomainCommand::UpdateChatDraft {
                                thread_id,
                                draft: message_draft,
                            },
                        )?;
                        apply_snapshot_command(
                            &mut workspace,
                            DomainCommand::SubmitChatDraft {
                                thread_id,
                                message_id: message.id(),
                                sent_at: message.sent_at(),
                            },
                        )?;
                    }
                    ChatAuthor::Agent(_) => apply_snapshot_command(
                        &mut workspace,
                        DomainCommand::AppendAgentChatMessage(message),
                    )?,
                }
            }
            if !draft.is_empty() {
                apply_snapshot_command(
                    &mut workspace,
                    DomainCommand::UpdateChatDraft { thread_id, draft },
                )?;
            }
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

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentProfileV1 {
    id: u64,
    name: String,
    kind: EnvironmentKindV1,
}

impl From<&EnvironmentProfile> for EnvironmentProfileV1 {
    fn from(profile: &EnvironmentProfile) -> Self {
        Self {
            id: profile.id().get(),
            name: profile.name().as_str().to_owned(),
            kind: EnvironmentKindV1::from(profile.kind()),
        }
    }
}

impl EnvironmentProfileV1 {
    fn into_domain(self) -> Result<EnvironmentProfile, String> {
        Ok(EnvironmentProfile::new(
            EnvironmentProfileId::new(self.id),
            Name::new(self.name).map_err(|error| error.to_string())?,
            self.kind.into_domain()?,
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum EnvironmentKindV1 {
    Ssh {
        host: String,
        user: Option<String>,
        port: Option<u16>,
        working_directory: String,
    },
    Container {
        engine: String,
        container: String,
        working_directory: String,
    },
    Custom {
        executable: String,
        arguments: Vec<String>,
        working_directory: String,
    },
}

impl From<&EnvironmentKind> for EnvironmentKindV1 {
    fn from(kind: &EnvironmentKind) -> Self {
        match kind {
            EnvironmentKind::Ssh(environment) => Self::Ssh {
                host: environment.host().to_owned(),
                user: environment.user().map(str::to_owned),
                port: environment.port(),
                working_directory: environment.working_directory().to_owned(),
            },
            EnvironmentKind::Container(environment) => Self::Container {
                engine: environment.engine().to_owned(),
                container: environment.container().to_owned(),
                working_directory: environment.working_directory().to_owned(),
            },
            EnvironmentKind::Custom(environment) => Self::Custom {
                executable: environment.executable().to_owned(),
                arguments: environment.arguments().to_vec(),
                working_directory: environment.working_directory().as_str().to_owned(),
            },
        }
    }
}

impl EnvironmentKindV1 {
    fn into_domain(self) -> Result<EnvironmentKind, String> {
        match self {
            Self::Ssh {
                host,
                user,
                port,
                working_directory,
            } => SshEnvironment::new(host, user, port, working_directory)
                .map(EnvironmentKind::Ssh)
                .map_err(|error| error.to_string()),
            Self::Container {
                engine,
                container,
                working_directory,
            } => ContainerEnvironment::new(engine, container, working_directory)
                .map(EnvironmentKind::Container)
                .map_err(|error| error.to_string()),
            Self::Custom {
                executable,
                arguments,
                working_directory,
            } => CustomEnvironment::new(
                executable,
                arguments,
                WorkspaceDirectory::new(working_directory).map_err(|error| error.to_string())?,
            )
            .map(EnvironmentKind::Custom)
            .map_err(|error| error.to_string()),
        }
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
struct CommandPresetV1 {
    id: u64,
    name: String,
    executable: String,
    arguments: Vec<String>,
}

impl From<&CommandPreset> for CommandPresetV1 {
    fn from(preset: &CommandPreset) -> Self {
        Self {
            id: preset.id().get(),
            name: preset.name().as_str().to_owned(),
            executable: preset.executable().to_owned(),
            arguments: preset.arguments().to_vec(),
        }
    }
}

impl CommandPresetV1 {
    fn into_domain(self) -> Result<CommandPreset, String> {
        CommandPreset::new(
            CommandPresetId::new(self.id),
            Name::new(self.name).map_err(|error| error.to_string())?,
            self.executable,
            self.arguments,
        )
        .map_err(|error| error.to_string())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleV1 {
    id: u64,
    name: String,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    instructions: String,
}

impl From<&Role> for RoleV1 {
    fn from(role: &Role) -> Self {
        Self {
            id: role.id().get(),
            name: role.name().as_str().to_owned(),
            color: Some(role.color().as_str().to_owned()),
            icon: Some(role.icon().as_str().to_owned()),
            instructions: role.instructions().as_str().to_owned(),
        }
    }
}

impl RoleV1 {
    fn has_appearance_fields(&self) -> bool {
        self.color.is_some() || self.icon.is_some()
    }

    fn into_domain(self) -> Result<Role, String> {
        Ok(Role::with_appearance(
            RoleId::new(self.id),
            Name::new(self.name).map_err(|error| error.to_string())?,
            self.color
                .map(RoleColor::new)
                .unwrap_or_else(|| RoleColor::new(RoleColor::DEFAULT))
                .map_err(|error| error.to_string())?,
            self.icon
                .map(RoleIcon::new)
                .unwrap_or_else(|| RoleIcon::new(RoleIcon::DEFAULT))
                .map_err(|error| error.to_string())?,
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
    #[serde(default)]
    environment_id: Option<u64>,
    state: AgentStateV1,
}

impl From<&Agent> for AgentV1 {
    fn from(agent: &Agent) -> Self {
        Self {
            id: agent.id().get(),
            name: agent.name().as_str().to_owned(),
            role_id: agent.role_id().map(RoleId::get),
            program: agent.program().into(),
            environment_id: agent.environment_id().map(EnvironmentProfileId::get),
            state: agent.state().into(),
        }
    }
}

impl AgentV1 {
    fn into_domain(self) -> Result<(Agent, AgentState), String> {
        let mut agent = Agent::with_program(
            AgentId::new(self.id),
            Name::new(self.name).map_err(|error| error.to_string())?,
            self.role_id.map(RoleId::new),
            self.program.into(),
        );
        if let Some(environment_id) = self.environment_id {
            agent = agent.in_environment(EnvironmentProfileId::new(environment_id));
        }
        Ok((agent, self.state.into()))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatThreadV1 {
    id: u64,
    agent_id: u64,
    name: String,
    color: String,
    draft: ChatDraftV1,
    messages: Vec<ChatMessageV1>,
}

impl From<&ChatThread> for ChatThreadV1 {
    fn from(thread: &ChatThread) -> Self {
        Self {
            id: thread.id().get(),
            agent_id: thread.agent_id().get(),
            name: thread.name().as_str().to_owned(),
            color: thread.color().as_str().to_owned(),
            draft: ChatDraftV1::from(thread.draft()),
            messages: thread.messages().iter().map(ChatMessageV1::from).collect(),
        }
    }
}

impl ChatThreadV1 {
    fn into_domain(self) -> Result<ChatThread, String> {
        let (thread, draft, messages) = self.into_parts()?;
        if !draft.is_empty() || !messages.is_empty() {
            return Err("a chat_thread_added event must contain an empty thread".to_owned());
        }
        Ok(thread)
    }

    fn into_parts(self) -> Result<(ChatThread, ChatDraft, Vec<ChatMessage>), String> {
        Ok((
            ChatThread::with_color(
                ChatThreadId::new(self.id),
                AgentId::new(self.agent_id),
                Name::new(self.name).map_err(|error| error.to_string())?,
                ThreadColor::new(self.color).map_err(|error| error.to_string())?,
            ),
            self.draft.into_domain()?,
            self.messages
                .into_iter()
                .map(ChatMessageV1::into_domain)
                .collect::<Result<_, _>>()?,
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatDraftV1 {
    text: String,
    attachments: Vec<u64>,
    mentions: Vec<NodeTargetV1>,
}

impl From<&ChatDraft> for ChatDraftV1 {
    fn from(draft: &ChatDraft) -> Self {
        Self {
            text: draft.text().to_owned(),
            attachments: draft
                .attachments()
                .iter()
                .copied()
                .map(ChatAttachmentId::get)
                .collect(),
            mentions: draft
                .mentions()
                .iter()
                .copied()
                .map(NodeTargetV1::from)
                .collect(),
        }
    }
}

impl ChatDraftV1 {
    fn into_domain(self) -> Result<ChatDraft, String> {
        ChatDraft::new(
            self.text,
            self.attachments
                .into_iter()
                .map(ChatAttachmentId::new)
                .collect(),
            self.mentions.into_iter().map(NodeTarget::from).collect(),
        )
        .map_err(|error| error.to_string())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatAttachmentV1 {
    id: u64,
    thread_id: u64,
    storage_key: String,
    display_name: String,
    media_type: String,
    byte_len: u64,
}

impl From<&ChatAttachment> for ChatAttachmentV1 {
    fn from(attachment: &ChatAttachment) -> Self {
        Self {
            id: attachment.id().get(),
            thread_id: attachment.thread_id().get(),
            storage_key: attachment.storage_key().to_owned(),
            display_name: attachment.display_name().to_owned(),
            media_type: attachment.media_type().to_owned(),
            byte_len: attachment.byte_len(),
        }
    }
}

impl ChatAttachmentV1 {
    fn into_domain(self) -> Result<ChatAttachment, String> {
        ChatAttachment::new(
            ChatAttachmentId::new(self.id),
            ChatThreadId::new(self.thread_id),
            self.storage_key,
            self.display_name,
            self.media_type,
            self.byte_len,
        )
        .map_err(|error| error.to_string())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatMessageV1 {
    id: u64,
    thread_id: u64,
    author: ChatAuthorV1,
    content: String,
    attachments: Vec<u64>,
    mentions: Vec<NodeTargetV1>,
    sent_at: u64,
}

impl From<&ChatMessage> for ChatMessageV1 {
    fn from(message: &ChatMessage) -> Self {
        Self {
            id: message.id().get(),
            thread_id: message.thread_id().get(),
            author: message.author().into(),
            content: message.content().as_str().to_owned(),
            attachments: message
                .attachments()
                .iter()
                .copied()
                .map(ChatAttachmentId::get)
                .collect(),
            mentions: message
                .mentions()
                .iter()
                .copied()
                .map(NodeTargetV1::from)
                .collect(),
            sent_at: message.sent_at().as_unix_millis(),
        }
    }
}

impl ChatMessageV1 {
    fn into_domain(self) -> Result<ChatMessage, String> {
        ChatMessage::new(
            ChatMessageId::new(self.id),
            ChatThreadId::new(self.thread_id),
            self.author.into(),
            Content::new(self.content).map_err(|error| error.to_string())?,
            self.attachments
                .into_iter()
                .map(ChatAttachmentId::new)
                .collect(),
            self.mentions.into_iter().map(NodeTarget::from).collect(),
            Timestamp::from_unix_millis(self.sent_at),
        )
        .map_err(|error| error.to_string())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ChatAuthorV1 {
    User,
    Agent { agent_id: u64 },
}

impl From<ChatAuthor> for ChatAuthorV1 {
    fn from(author: ChatAuthor) -> Self {
        match author {
            ChatAuthor::User => Self::User,
            ChatAuthor::Agent(agent_id) => Self::Agent {
                agent_id: agent_id.get(),
            },
        }
    }
}

impl From<ChatAuthorV1> for ChatAuthor {
    fn from(author: ChatAuthorV1) -> Self {
        match author {
            ChatAuthorV1::User => Self::User,
            ChatAuthorV1::Agent { agent_id } => Self::Agent(AgentId::new(agent_id)),
        }
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
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    parent: Option<u64>,
    #[serde(default)]
    created_at: Option<u64>,
    #[serde(default)]
    response_deadline: Option<u64>,
    #[serde(default)]
    delivery_attempts: Vec<DeliveryAttemptV1>,
    #[serde(default)]
    progress: Vec<HandoffProgressV1>,
    #[serde(default)]
    response: Option<HandoffResponseV1>,
    #[serde(default)]
    termination: Option<HandoffTerminationV1>,
}

impl From<&Handoff> for HandoffV1 {
    fn from(handoff: &Handoff) -> Self {
        Self {
            id: handoff.id().get(),
            source: handoff.source().get(),
            recipient: handoff.recipient().get(),
            payload: HandoffPayloadV1::from(handoff.payload()),
            message_id: handoff
                .message_id()
                .map(|message_id| message_id.as_str().to_owned()),
            parent: handoff.parent().map(HandoffId::get),
            created_at: handoff.created_at().map(Timestamp::as_unix_millis),
            response_deadline: handoff.response_deadline().map(Timestamp::as_unix_millis),
            delivery_attempts: handoff
                .delivery_attempts()
                .iter()
                .map(DeliveryAttemptV1::from)
                .collect(),
            progress: handoff
                .progress()
                .iter()
                .map(HandoffProgressV1::from)
                .collect(),
            response: handoff.response().map(HandoffResponseV1::from),
            termination: handoff.termination().map(HandoffTerminationV1::from),
        }
    }
}

impl HandoffV1 {
    fn into_domain(self) -> Result<Handoff, String> {
        let id = HandoffId::new(self.id);
        let source = AgentId::new(self.source);
        let recipient = AgentId::new(self.recipient);
        let payload = self.payload.into_domain()?;
        let Some(message_id) = self.message_id else {
            if self.parent.is_some()
                || self.created_at.is_some()
                || self.response_deadline.is_some()
                || !self.delivery_attempts.is_empty()
                || !self.progress.is_empty()
                || self.response.is_some()
                || self.termination.is_some()
            {
                return Err("legacy handoff contains orchestration fields".to_owned());
            }
            return Ok(Handoff::new(id, source, recipient, payload));
        };
        let created_at = self
            .created_at
            .map(Timestamp::from_unix_millis)
            .ok_or_else(|| "tracked handoff is missing created_at".to_owned())?;
        let root_message_id =
            HandoffMessageId::new(message_id).map_err(|error| error.to_string())?;
        let mut handoff = Handoff::tracked(
            id,
            root_message_id.clone(),
            source,
            recipient,
            payload,
            self.parent.map(HandoffId::new),
            created_at,
            self.response_deadline.map(Timestamp::from_unix_millis),
        )
        .map_err(|error| error.to_string())?;

        let (root_attempts, follow_up_attempts): (Vec<_>, Vec<_>) = self
            .delivery_attempts
            .into_iter()
            .partition(|attempt| attempt.message_id == root_message_id.as_str());
        restore_delivery_attempts(&mut handoff, root_attempts)?;
        for progress in self.progress {
            handoff
                .report_progress(progress.into_domain()?)
                .map_err(|error| error.to_string())?;
        }
        if let Some(response) = self.response {
            handoff
                .respond(response.into_domain()?)
                .map_err(|error| error.to_string())?;
        }
        if let Some(termination) = self.termination {
            match termination {
                HandoffTerminationV1::Cancelled {
                    message_id,
                    reason,
                    cancelled_at,
                } => handoff
                    .cancel(
                        HandoffMessageId::new(message_id).map_err(|error| error.to_string())?,
                        Content::new(reason).map_err(|error| error.to_string())?,
                        Timestamp::from_unix_millis(cancelled_at),
                    )
                    .map_err(|error| error.to_string())?,
                HandoffTerminationV1::TimedOut { timed_out_at } => handoff
                    .time_out(Timestamp::from_unix_millis(timed_out_at))
                    .map_err(|error| error.to_string())?,
            }
        }
        restore_delivery_attempts(&mut handoff, follow_up_attempts)?;
        Ok(handoff)
    }

    fn has_orchestration_fields(&self) -> bool {
        self.message_id.is_some()
            || self.parent.is_some()
            || self.created_at.is_some()
            || self.response_deadline.is_some()
            || !self.delivery_attempts.is_empty()
            || !self.progress.is_empty()
            || self.response.is_some()
            || self.termination.is_some()
    }
}

fn restore_delivery_attempts(
    handoff: &mut Handoff,
    attempts: impl IntoIterator<Item = DeliveryAttemptV1>,
) -> Result<(), String> {
    for attempt in attempts {
        let ordinal = handoff
            .begin_delivery(
                HandoffMessageId::new(attempt.message_id).map_err(|error| error.to_string())?,
                attempt.mechanism.into(),
                Timestamp::from_unix_millis(attempt.started_at),
            )
            .map_err(|error| error.to_string())?;
        if ordinal != attempt.ordinal {
            return Err("delivery attempt ordinals are not contiguous".to_owned());
        }
        match attempt.outcome {
            DeliveryOutcomeV1::Started => {}
            DeliveryOutcomeV1::Delivered { finished_at } => handoff
                .complete_delivery(ordinal, Timestamp::from_unix_millis(finished_at))
                .map_err(|error| error.to_string())?,
            DeliveryOutcomeV1::Failed {
                finished_at,
                error,
                retryable,
            } => handoff
                .fail_delivery(
                    ordinal,
                    Timestamp::from_unix_millis(finished_at),
                    Content::new(error).map_err(|error| error.to_string())?,
                    retryable,
                )
                .map_err(|error| error.to_string())?,
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeliveryAttemptV1 {
    ordinal: u32,
    message_id: String,
    started_at: u64,
    mechanism: DeliveryMechanismV1,
    outcome: DeliveryOutcomeV1,
}

impl From<&DeliveryAttempt> for DeliveryAttemptV1 {
    fn from(attempt: &DeliveryAttempt) -> Self {
        Self {
            ordinal: attempt.ordinal(),
            message_id: attempt.message_id().as_str().to_owned(),
            started_at: attempt.started_at().as_unix_millis(),
            mechanism: attempt.mechanism().into(),
            outcome: DeliveryOutcomeV1::from(attempt.outcome()),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DeliveryMechanismV1 {
    #[serde(rename = "codex_terminal")]
    Codex,
    #[serde(rename = "claude_terminal")]
    Claude,
    #[serde(rename = "open_code_terminal")]
    OpenCode,
}

impl From<DeliveryMechanism> for DeliveryMechanismV1 {
    fn from(mechanism: DeliveryMechanism) -> Self {
        match mechanism {
            DeliveryMechanism::CodexTerminal => Self::Codex,
            DeliveryMechanism::ClaudeTerminal => Self::Claude,
            DeliveryMechanism::OpenCodeTerminal => Self::OpenCode,
        }
    }
}

impl From<DeliveryMechanismV1> for DeliveryMechanism {
    fn from(mechanism: DeliveryMechanismV1) -> Self {
        match mechanism {
            DeliveryMechanismV1::Codex => Self::CodexTerminal,
            DeliveryMechanismV1::Claude => Self::ClaudeTerminal,
            DeliveryMechanismV1::OpenCode => Self::OpenCodeTerminal,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum DeliveryOutcomeV1 {
    Started,
    Delivered {
        finished_at: u64,
    },
    Failed {
        finished_at: u64,
        error: String,
        #[serde(default = "default_true")]
        retryable: bool,
    },
}

impl From<&DeliveryOutcome> for DeliveryOutcomeV1 {
    fn from(outcome: &DeliveryOutcome) -> Self {
        match outcome {
            DeliveryOutcome::Started => Self::Started,
            DeliveryOutcome::Delivered { finished_at } => Self::Delivered {
                finished_at: finished_at.as_unix_millis(),
            },
            DeliveryOutcome::Failed {
                finished_at,
                error,
                retryable,
            } => Self::Failed {
                finished_at: finished_at.as_unix_millis(),
                error: error.as_str().to_owned(),
                retryable: *retryable,
            },
        }
    }
}

const fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HandoffProgressV1 {
    message_id: String,
    body: String,
    reported_at: u64,
}

impl From<&HandoffProgress> for HandoffProgressV1 {
    fn from(progress: &HandoffProgress) -> Self {
        Self {
            message_id: progress.message_id().as_str().to_owned(),
            body: progress.body().as_str().to_owned(),
            reported_at: progress.reported_at().as_unix_millis(),
        }
    }
}

impl HandoffProgressV1 {
    fn into_domain(self) -> Result<HandoffProgress, String> {
        Ok(HandoffProgress::new(
            HandoffMessageId::new(self.message_id).map_err(|error| error.to_string())?,
            Content::new(self.body).map_err(|error| error.to_string())?,
            Timestamp::from_unix_millis(self.reported_at),
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HandoffResponseV1 {
    message_id: String,
    status: HandoffResponseStatusV1,
    body: String,
    responded_at: u64,
}

impl From<&HandoffResponse> for HandoffResponseV1 {
    fn from(response: &HandoffResponse) -> Self {
        Self {
            message_id: response.message_id().as_str().to_owned(),
            status: response.status().into(),
            body: response.body().as_str().to_owned(),
            responded_at: response.responded_at().as_unix_millis(),
        }
    }
}

impl HandoffResponseV1 {
    fn into_domain(self) -> Result<HandoffResponse, String> {
        Ok(HandoffResponse::new(
            HandoffMessageId::new(self.message_id).map_err(|error| error.to_string())?,
            self.status.into(),
            Content::new(self.body).map_err(|error| error.to_string())?,
            Timestamp::from_unix_millis(self.responded_at),
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum HandoffResponseStatusV1 {
    Completed,
    Failed,
    Blocked,
}

impl From<HandoffResponseStatus> for HandoffResponseStatusV1 {
    fn from(status: HandoffResponseStatus) -> Self {
        match status {
            HandoffResponseStatus::Completed => Self::Completed,
            HandoffResponseStatus::Failed => Self::Failed,
            HandoffResponseStatus::Blocked => Self::Blocked,
        }
    }
}

impl From<HandoffResponseStatusV1> for HandoffResponseStatus {
    fn from(status: HandoffResponseStatusV1) -> Self {
        match status {
            HandoffResponseStatusV1::Completed => Self::Completed,
            HandoffResponseStatusV1::Failed => Self::Failed,
            HandoffResponseStatusV1::Blocked => Self::Blocked,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum HandoffTerminationV1 {
    Cancelled {
        message_id: String,
        reason: String,
        cancelled_at: u64,
    },
    TimedOut {
        timed_out_at: u64,
    },
}

impl From<&HandoffTermination> for HandoffTerminationV1 {
    fn from(termination: &HandoffTermination) -> Self {
        match termination {
            HandoffTermination::Cancelled {
                message_id,
                reason,
                cancelled_at,
            } => Self::Cancelled {
                message_id: message_id.as_str().to_owned(),
                reason: reason.as_str().to_owned(),
                cancelled_at: cancelled_at.as_unix_millis(),
            },
            HandoffTermination::TimedOut { timed_out_at } => Self::TimedOut {
                timed_out_at: timed_out_at.as_unix_millis(),
            },
        }
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
    OpenCode,
    Custom {
        preset_id: u64,
    },
    #[default]
    Shell,
}

impl From<AgentProgram> for AgentProgramV1 {
    fn from(program: AgentProgram) -> Self {
        match program {
            AgentProgram::Codex => Self::Codex,
            AgentProgram::Claude => Self::Claude,
            AgentProgram::OpenCode => Self::OpenCode,
            AgentProgram::Custom(preset_id) => Self::Custom {
                preset_id: preset_id.get(),
            },
            AgentProgram::Shell => Self::Shell,
        }
    }
}

impl From<AgentProgramV1> for AgentProgram {
    fn from(program: AgentProgramV1) -> Self {
        match program {
            AgentProgramV1::Codex => Self::Codex,
            AgentProgramV1::Claude => Self::Claude,
            AgentProgramV1::OpenCode => Self::OpenCode,
            AgentProgramV1::Custom { preset_id } => Self::Custom(CommandPresetId::new(preset_id)),
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
