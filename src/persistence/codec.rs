use serde::{Deserialize, Serialize};

use crate::domain::{
    Agent, AgentId, AgentProgram, AgentState, Arrow, CanvasColor, CanvasLayout, CanvasNodeContent,
    CanvasPoint, CanvasSize, CanvasText, ChatAttachment, ChatAttachmentId, ChatAuthor, ChatDraft,
    ChatMessage, ChatMessageId, ChatThread, ChatThreadId, CommandPreset, CommandPresetId,
    Connection, ConnectionId, ConnectionKind, ContainerEnvironment, Content, CustomEnvironment,
    DeliveryAttempt, DeliveryMechanism, DeliveryOutcome, DiffComparison, DomainCommand,
    DomainEvent, EnvironmentKind, EnvironmentProfile, EnvironmentProfileId, Freehand, Handoff,
    HandoffId, HandoffMessageId, HandoffOrigin, HandoffPayload, HandoffProgress, HandoffResponse,
    HandoffResponseStatus, HandoffTermination, Name, Node, NodeGroup, NodeGroupId, NodeId,
    NodeTarget, NormalizedPoint, PortalConfig, PortalPresentation, PortalTarget, PortalTargetKind,
    ProjectPath, Role, RoleColor, RoleIcon, RoleId, Routine, RoutineApproval,
    RoutineApprovalDecision, RoutineApprovalRecord, RoutineAttempt, RoutineAttemptId,
    RoutineAttemptOutcome, RoutineBindingSource, RoutineCadence, RoutineCheckout,
    RoutineCheckoutClaim, RoutineId, RoutineInputDeclaration, RoutineInputKey, RoutineInterruption,
    RoutineInterruptionReason, RoutineOccurrenceKey, RoutineOutputKey, RoutineResourceKey,
    RoutineRetryPolicy, RoutineRun, RoutineRunId, RoutineRunPin, RoutineRunState, RoutineSchedule,
    RoutineStep, RoutineStepClaims, RoutineStepId, RoutineStepRun, RoutineStepState,
    RoutineTransition, RoutineTrigger, RoutineTriggerFiring, RoutineTriggerId, RoutineTriggerKind,
    RoutineValue, RoutineVersion, RoutineVersionId, Shape, ShapeKind, SshEnvironment, StrokeWidth,
    Task, TaskId, TaskState, ThreadColor, Timestamp, Workspace, WorkspaceDirectory, WorkspaceIcon,
    WorkspaceId, WorkspaceSettings,
};

use super::PersistenceError;

pub(crate) const EVENT_FORMAT_VERSION: u32 = 12;
pub(crate) const SNAPSHOT_FORMAT_VERSION: u32 = 11;

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

    if format_version < 9 && matches!(stored, StoredEvent::FloorsChanged { .. }) {
        return Err(PersistenceError::invalid_record(
            "domain event",
            sequence,
            "floors require event format version 9",
        ));
    }
    if format_version < 10 && stored.requires_version_ten() {
        return Err(PersistenceError::invalid_record(
            "domain event",
            sequence,
            "canvas context and drawing nodes require event format version 10",
        ));
    }
    if format_version < 11 && stored.requires_version_eleven() {
        return Err(PersistenceError::invalid_record(
            "domain event",
            sequence,
            "portal canvas nodes require event format version 11",
        ));
    }
    if format_version < 12 && stored.requires_version_twelve() {
        return Err(PersistenceError::invalid_record(
            "domain event",
            sequence,
            "routines, routine runs, and structured step outputs require event format version 12",
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
    FloorsChanged {
        before: FloorsV1,
        after: FloorsV1,
    },
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
    RoutineAdded {
        routine: RoutineV1,
    },
    RoutineVersionAdded {
        routine_id: u64,
        version: RoutineVersionV1,
    },
    RoutineChanged {
        routine_id: u64,
        from_name: String,
        to_name: String,
        from_description: Option<String>,
        to_description: Option<String>,
    },
    RoutineTriggerChanged {
        routine_id: u64,
        from: Option<RoutineTriggerV1>,
        to: RoutineTriggerV1,
    },
    RoutineTriggerRemoved {
        routine_id: u64,
        trigger: RoutineTriggerV1,
    },
    RoutineRunStarted {
        run: RoutineRunV1,
        from_firing: Option<RoutineFiringV1>,
        next_occurrence: Option<u64>,
    },
    RoutineOccurrencesSkipped {
        routine_id: u64,
        trigger_id: u64,
        skipped: u32,
        next_occurrence: Option<u64>,
    },
    RoutineRunAdvanced {
        run_id: u64,
        transition: RoutineTransitionV1,
    },
    RoutineStepDispatched {
        run_id: u64,
        step_id: u64,
        attempt: RoutineAttemptV1,
        task: TaskV1,
        handoff: HandoffV1,
    },
}

impl From<&DomainEvent> for StoredEvent {
    fn from(event: &DomainEvent) -> Self {
        match event {
            DomainEvent::FloorsChanged { before, after } => Self::FloorsChanged {
                before: before.into(),
                after: after.into(),
            },
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
            DomainEvent::RoutineAdded(routine) => Self::RoutineAdded {
                routine: RoutineV1::from(routine),
            },
            DomainEvent::RoutineVersionAdded {
                routine_id,
                version,
            } => Self::RoutineVersionAdded {
                routine_id: routine_id.get(),
                version: RoutineVersionV1::from(version),
            },
            DomainEvent::RoutineChanged {
                routine_id,
                from_name,
                to_name,
                from_description,
                to_description,
            } => Self::RoutineChanged {
                routine_id: routine_id.get(),
                from_name: from_name.as_str().to_owned(),
                to_name: to_name.as_str().to_owned(),
                from_description: from_description
                    .as_ref()
                    .map(|value| value.as_str().to_owned()),
                to_description: to_description
                    .as_ref()
                    .map(|value| value.as_str().to_owned()),
            },
            DomainEvent::RoutineTriggerChanged {
                routine_id,
                from,
                to,
            } => Self::RoutineTriggerChanged {
                routine_id: routine_id.get(),
                from: from.as_ref().map(RoutineTriggerV1::from),
                to: RoutineTriggerV1::from(to),
            },
            DomainEvent::RoutineTriggerRemoved {
                routine_id,
                trigger,
            } => Self::RoutineTriggerRemoved {
                routine_id: routine_id.get(),
                trigger: RoutineTriggerV1::from(trigger),
            },
            DomainEvent::RoutineRunStarted {
                run,
                firing,
                next_occurrence,
            } => Self::RoutineRunStarted {
                run: RoutineRunV1::from(run),
                from_firing: firing.as_ref().map(RoutineFiringV1::from),
                next_occurrence: next_occurrence.map(Timestamp::as_unix_millis),
            },
            DomainEvent::RoutineOccurrencesSkipped {
                routine_id,
                trigger_id,
                skipped,
                next_occurrence,
            } => Self::RoutineOccurrencesSkipped {
                routine_id: routine_id.get(),
                trigger_id: trigger_id.get(),
                skipped: *skipped,
                next_occurrence: next_occurrence.map(Timestamp::as_unix_millis),
            },
            DomainEvent::RoutineRunAdvanced { run_id, transition } => Self::RoutineRunAdvanced {
                run_id: run_id.get(),
                transition: RoutineTransitionV1::from(transition),
            },
            DomainEvent::RoutineStepDispatched {
                run_id,
                step_id,
                attempt,
                task,
                handoff,
            } => Self::RoutineStepDispatched {
                run_id: run_id.get(),
                step_id: step_id.get(),
                attempt: RoutineAttemptV1::from(attempt),
                task: TaskV1::from(task),
                handoff: HandoffV1::from(handoff),
            },
        }
    }
}

impl StoredEvent {
    /// Routine records and the structured outputs a routine step returns.
    fn requires_version_twelve(&self) -> bool {
        match self {
            Self::RoutineAdded { .. }
            | Self::RoutineVersionAdded { .. }
            | Self::RoutineChanged { .. }
            | Self::RoutineTriggerChanged { .. }
            | Self::RoutineTriggerRemoved { .. }
            | Self::RoutineRunStarted { .. }
            | Self::RoutineOccurrencesSkipped { .. }
            | Self::RoutineRunAdvanced { .. }
            | Self::RoutineStepDispatched { .. } => true,
            Self::HandoffAdded { handoff } | Self::TaskHandoffAdded { handoff, .. } => {
                handoff.uses_routine_fields()
            }
            Self::HandoffChanged { after, .. } => after.uses_routine_fields(),
            Self::TaskCancelled { after, .. } => after.uses_routine_fields(),
            Self::TaskResumed { handoff, .. } => handoff.uses_routine_fields(),
            _ => false,
        }
    }

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

    fn requires_version_ten(&self) -> bool {
        match self {
            Self::NodeAdded { node } | Self::AgentNodeAdded { node, .. } => node.content.is_some(),
            Self::CanvasReplaced { before, after } => {
                before.has_owned_content() || after.has_owned_content()
            }
            _ => false,
        }
    }

    fn requires_version_eleven(&self) -> bool {
        match self {
            Self::NodeAdded { node } | Self::AgentNodeAdded { node, .. } => {
                node.has_portal_content()
            }
            Self::CanvasReplaced { before, after } => {
                before.has_portal_content() || after.has_portal_content()
            }
            _ => false,
        }
    }

    fn into_domain(self) -> Result<DomainEvent, String> {
        match self {
            Self::FloorsChanged { before, after } => Ok(DomainEvent::FloorsChanged {
                before: before.into_domain()?,
                after: after.into_domain()?,
            }),
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
            Self::RoutineAdded { routine } => Ok(DomainEvent::RoutineAdded(routine.into_domain()?)),
            Self::RoutineVersionAdded {
                routine_id,
                version,
            } => Ok(DomainEvent::RoutineVersionAdded {
                routine_id: RoutineId::new(routine_id),
                version: version.into_domain()?,
            }),
            Self::RoutineChanged {
                routine_id,
                from_name,
                to_name,
                from_description,
                to_description,
            } => Ok(DomainEvent::RoutineChanged {
                routine_id: RoutineId::new(routine_id),
                from_name: Name::new(from_name).map_err(|error| error.to_string())?,
                to_name: Name::new(to_name).map_err(|error| error.to_string())?,
                from_description: from_description
                    .map(Content::new)
                    .transpose()
                    .map_err(|error| error.to_string())?,
                to_description: to_description
                    .map(Content::new)
                    .transpose()
                    .map_err(|error| error.to_string())?,
            }),
            Self::RoutineTriggerChanged {
                routine_id,
                from,
                to,
            } => Ok(DomainEvent::RoutineTriggerChanged {
                routine_id: RoutineId::new(routine_id),
                from: from.map(RoutineTriggerV1::into_domain).transpose()?,
                to: to.into_domain()?,
            }),
            Self::RoutineTriggerRemoved {
                routine_id,
                trigger,
            } => Ok(DomainEvent::RoutineTriggerRemoved {
                routine_id: RoutineId::new(routine_id),
                trigger: trigger.into_domain()?,
            }),
            Self::RoutineRunStarted {
                run,
                from_firing,
                next_occurrence,
            } => Ok(DomainEvent::RoutineRunStarted {
                run: run.into_domain()?,
                firing: from_firing.map(RoutineFiringV1::into_domain).transpose()?,
                next_occurrence: next_occurrence.map(Timestamp::from_unix_millis),
            }),
            Self::RoutineOccurrencesSkipped {
                routine_id,
                trigger_id,
                skipped,
                next_occurrence,
            } => Ok(DomainEvent::RoutineOccurrencesSkipped {
                routine_id: RoutineId::new(routine_id),
                trigger_id: RoutineTriggerId::new(trigger_id),
                skipped,
                next_occurrence: next_occurrence.map(Timestamp::from_unix_millis),
            }),
            Self::RoutineRunAdvanced { run_id, transition } => {
                Ok(DomainEvent::RoutineRunAdvanced {
                    run_id: RoutineRunId::new(run_id),
                    transition: transition.into_domain()?,
                })
            }
            Self::RoutineStepDispatched {
                run_id,
                step_id,
                attempt,
                task,
                handoff,
            } => {
                let (task, state) = task.into_domain()?;
                if state != TaskState::Queued {
                    return Err("a dispatched routine step records a queued task".to_owned());
                }
                Ok(DomainEvent::RoutineStepDispatched {
                    run_id: RoutineRunId::new(run_id),
                    step_id: RoutineStepId::new(step_id),
                    attempt: attempt.into_domain()?,
                    task,
                    handoff: handoff.into_domain()?,
                })
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredWorkspace {
    #[serde(default)]
    floors: FloorsV1,
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
    #[serde(default)]
    routines: Vec<RoutineV1>,
    #[serde(default)]
    routine_runs: Vec<RoutineRunV1>,
}

impl From<&Workspace> for StoredWorkspace {
    fn from(workspace: &Workspace) -> Self {
        Self {
            floors: workspace.floors().into(),
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
            routines: workspace.routines().map(RoutineV1::from).collect(),
            routine_runs: workspace.routine_runs().map(RoutineRunV1::from).collect(),
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
        if format_version < 9 && self.nodes.iter().any(|node| node.content.is_some()) {
            return Err(
                "canvas context and drawing nodes require snapshot format version 9".to_owned(),
            );
        }
        if format_version < 10 && self.nodes.iter().any(NodeV1::has_portal_content) {
            return Err("portal canvas nodes require snapshot format version 10".to_owned());
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

        if format_version < 11 && (!self.routines.is_empty() || !self.routine_runs.is_empty()) {
            return Err("routines require snapshot format version 11".to_owned());
        }
        for routine in self.routines {
            apply_snapshot_command(
                &mut workspace,
                DomainCommand::AddRoutine(routine.into_domain()?),
            )?;
        }
        for run in self.routine_runs {
            workspace
                .restore_routine_run(run.into_domain()?)
                .map_err(|error| error.to_string())?;
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

        let floors = self.floors.into_domain()?;
        if floors != crate::domain::Floors::default() {
            if format_version < 8 {
                return Err("floors require snapshot format version 8".to_owned());
            }
            let before = workspace.floors().clone();
            apply_snapshot_command(
                &mut workspace,
                DomainCommand::ReplaceFloors {
                    before,
                    after: floors,
                },
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
    /// Zero when the routine scheduler submitted the handoff; `routine_origin`
    /// then names the run and step instead of an agent.
    source: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    routine_origin: Option<RoutineOriginV1>,
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
        let (source, routine_origin) = match handoff.origin() {
            HandoffOrigin::Agent(agent_id) => (agent_id.get(), None),
            HandoffOrigin::Routine { run_id, step_id } => (
                0,
                Some(RoutineOriginV1 {
                    run_id: run_id.get(),
                    step_id: step_id.get(),
                }),
            ),
        };
        Self {
            id: handoff.id().get(),
            source,
            routine_origin,
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
        let origin = match self.routine_origin {
            Some(origin) => HandoffOrigin::Routine {
                run_id: RoutineRunId::new(origin.run_id),
                step_id: RoutineStepId::new(origin.step_id),
            },
            None => HandoffOrigin::Agent(AgentId::new(self.source)),
        };
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
            return Ok(Handoff::with_origin(id, origin, recipient, payload));
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
            origin,
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

    /// Whether the record needs the routine-aware event format.
    fn uses_routine_fields(&self) -> bool {
        self.routine_origin.is_some()
            || self
                .response
                .as_ref()
                .is_some_and(|response| !response.outputs.is_empty())
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    outputs: Vec<RoutineValueEntryV1>,
}

impl From<&HandoffResponse> for HandoffResponseV1 {
    fn from(response: &HandoffResponse) -> Self {
        Self {
            message_id: response.message_id().as_str().to_owned(),
            status: response.status().into(),
            body: response.body().as_str().to_owned(),
            responded_at: response.responded_at().as_unix_millis(),
            outputs: response
                .outputs()
                .iter()
                .map(|(key, value)| RoutineValueEntryV1 {
                    key: key.as_str().to_owned(),
                    value: value.as_str().to_owned(),
                })
                .collect(),
        }
    }
}

impl HandoffResponseV1 {
    fn into_domain(self) -> Result<HandoffResponse, String> {
        let outputs = self
            .outputs
            .into_iter()
            .map(|entry| {
                Ok((
                    RoutineOutputKey::new(entry.key)
                        .map_err(|e: crate::domain::RoutineError| e.to_string())?,
                    RoutineValue::new(entry.value).map_err(|e| e.to_string())?,
                ))
            })
            .collect::<Result<_, String>>()?;
        Ok(HandoffResponse::new(
            HandoffMessageId::new(self.message_id).map_err(|error| error.to_string())?,
            self.status.into(),
            Content::new(self.body).map_err(|error| error.to_string())?,
            Timestamp::from_unix_millis(self.responded_at),
        )
        .with_outputs(outputs))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineOriginV1 {
    run_id: u64,
    step_id: u64,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<NodeTargetV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content: Option<CanvasNodeContentV1>,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    #[serde(default)]
    z_index: i32,
}

impl From<&Node> for NodeV1 {
    fn from(node: &Node) -> Self {
        let (target, content) = match node.content() {
            CanvasNodeContent::Reference(target) => (Some((*target).into()), None),
            content => (None, Some(content.into())),
        };
        Self {
            id: node.id().get(),
            target,
            content,
            x: node.position().x(),
            y: node.position().y(),
            width: node.size().width(),
            height: node.size().height(),
            z_index: node.z_index(),
        }
    }
}

impl NodeV1 {
    fn has_portal_content(&self) -> bool {
        self.content
            .as_ref()
            .is_some_and(CanvasNodeContentV1::is_portal)
    }

    fn into_domain(self) -> Result<Node, String> {
        let content = match (self.target, self.content) {
            (Some(target), None) => CanvasNodeContent::Reference(target.into()),
            (None, Some(content)) => content.into_domain()?,
            (Some(_), Some(_)) => {
                return Err("a stored node cannot have both a target and owned content".to_owned());
            }
            (None, None) => return Err("a stored node must have content".to_owned()),
        };
        Ok(Node::with_content_and_z_index(
            NodeId::new(self.id),
            content,
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
    Reference,
}

impl From<ConnectionKind> for ConnectionKindV1 {
    fn from(kind: ConnectionKind) -> Self {
        match kind {
            ConnectionKind::Coordination => Self::Coordination,
            ConnectionKind::Assignment => Self::Assignment,
            ConnectionKind::Dependency => Self::Dependency,
            ConnectionKind::Handoff => Self::Handoff,
            ConnectionKind::Reference => Self::Reference,
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
            ConnectionKindV1::Reference => Self::Reference,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CanvasLayoutV1 {
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
    fn has_owned_content(&self) -> bool {
        self.nodes.iter().any(|node| node.content.is_some())
    }

    fn has_portal_content(&self) -> bool {
        self.nodes.iter().any(NodeV1::has_portal_content)
    }

    pub(super) fn into_domain(self) -> Result<CanvasLayout, String> {
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
enum CanvasNodeContentV1 {
    Note {
        path: String,
        title: String,
    },
    FileTree {
        root: String,
    },
    Artifact {
        path: String,
    },
    Diff {
        path: String,
        comparison: DiffComparisonV1,
    },
    Text {
        markdown: String,
    },
    Portal {
        kind: PortalTargetKindV1,
        selector: String,
        preserve_aspect_ratio: bool,
        frame_rate_limit: u16,
    },
    Shape {
        kind: ShapeKindV1,
        fill: [u8; 4],
        stroke: [u8; 4],
        stroke_width: f32,
    },
    Arrow {
        start: NormalizedPointV1,
        end: NormalizedPointV1,
        stroke: [u8; 4],
        stroke_width: f32,
        label: Option<String>,
    },
    Freehand {
        points: Vec<NormalizedPointV1>,
        stroke: [u8; 4],
        stroke_width: f32,
    },
}

impl From<&CanvasNodeContent> for CanvasNodeContentV1 {
    fn from(content: &CanvasNodeContent) -> Self {
        match content {
            CanvasNodeContent::Reference(_) => {
                unreachable!("reference node content is stored in the legacy target field")
            }
            CanvasNodeContent::Note { path, title } => Self::Note {
                path: path.as_str().to_owned(),
                title: title.as_str().to_owned(),
            },
            CanvasNodeContent::FileTree { root } => Self::FileTree {
                root: root.as_str().to_owned(),
            },
            CanvasNodeContent::Artifact { path } => Self::Artifact {
                path: path.as_str().to_owned(),
            },
            CanvasNodeContent::Diff { path, comparison } => Self::Diff {
                path: path.as_str().to_owned(),
                comparison: (*comparison).into(),
            },
            CanvasNodeContent::Text { markdown } => Self::Text {
                markdown: markdown.as_str().to_owned(),
            },
            CanvasNodeContent::Portal(config) => Self::Portal {
                kind: config.target().kind().into(),
                selector: config.target().selector().to_owned(),
                preserve_aspect_ratio: config.presentation().preserve_aspect_ratio(),
                frame_rate_limit: config.presentation().frame_rate_limit(),
            },
            CanvasNodeContent::Shape(shape) => Self::Shape {
                kind: shape.kind().into(),
                fill: shape.fill().channels(),
                stroke: shape.stroke().channels(),
                stroke_width: shape.stroke_width().get(),
            },
            CanvasNodeContent::Arrow(arrow) => Self::Arrow {
                start: arrow.start().into(),
                end: arrow.end().into(),
                stroke: arrow.stroke().channels(),
                stroke_width: arrow.stroke_width().get(),
                label: arrow.label().map(|label| label.as_str().to_owned()),
            },
            CanvasNodeContent::Freehand(freehand) => Self::Freehand {
                points: freehand.points().iter().copied().map(Into::into).collect(),
                stroke: freehand.stroke().channels(),
                stroke_width: freehand.stroke_width().get(),
            },
        }
    }
}

impl CanvasNodeContentV1 {
    fn is_portal(&self) -> bool {
        matches!(self, Self::Portal { .. })
    }

    fn into_domain(self) -> Result<CanvasNodeContent, String> {
        match self {
            Self::Note { path, title } => Ok(CanvasNodeContent::Note {
                path: project_path(path)?,
                title: Name::new(title).map_err(|error| error.to_string())?,
            }),
            Self::FileTree { root } => Ok(CanvasNodeContent::FileTree {
                root: project_path(root)?,
            }),
            Self::Artifact { path } => Ok(CanvasNodeContent::Artifact {
                path: project_path(path)?,
            }),
            Self::Diff { path, comparison } => Ok(CanvasNodeContent::Diff {
                path: project_path(path)?,
                comparison: comparison.into(),
            }),
            Self::Text { markdown } => Ok(CanvasNodeContent::Text {
                markdown: CanvasText::new(markdown).map_err(|error| error.to_string())?,
            }),
            Self::Portal {
                kind,
                selector,
                preserve_aspect_ratio,
                frame_rate_limit,
            } => {
                let target =
                    PortalTarget::new(kind.into(), selector).map_err(|error| error.to_string())?;
                let presentation = PortalPresentation::new(preserve_aspect_ratio, frame_rate_limit)
                    .ok_or_else(|| {
                        "portal frame rate limit must be greater than zero".to_owned()
                    })?;
                Ok(CanvasNodeContent::Portal(PortalConfig::new(
                    target,
                    presentation,
                )))
            }
            Self::Shape {
                kind,
                fill,
                stroke,
                stroke_width,
            } => Ok(CanvasNodeContent::Shape(Shape::new(
                kind.into(),
                color(fill),
                color(stroke),
                StrokeWidth::new(stroke_width).map_err(|error| error.to_string())?,
            ))),
            Self::Arrow {
                start,
                end,
                stroke,
                stroke_width,
                label,
            } => Ok(CanvasNodeContent::Arrow(Arrow::new(
                start.into_domain()?,
                end.into_domain()?,
                color(stroke),
                StrokeWidth::new(stroke_width).map_err(|error| error.to_string())?,
                label
                    .map(Name::new)
                    .transpose()
                    .map_err(|error| error.to_string())?,
            ))),
            Self::Freehand {
                points,
                stroke,
                stroke_width,
            } => Ok(CanvasNodeContent::Freehand(
                Freehand::new(
                    points
                        .into_iter()
                        .map(NormalizedPointV1::into_domain)
                        .collect::<Result<_, _>>()?,
                    color(stroke),
                    StrokeWidth::new(stroke_width).map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?,
            )),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PortalTargetKindV1 {
    Browser,
    Android,
    Ios,
}

impl From<PortalTargetKind> for PortalTargetKindV1 {
    fn from(kind: PortalTargetKind) -> Self {
        match kind {
            PortalTargetKind::Browser => Self::Browser,
            PortalTargetKind::Android => Self::Android,
            PortalTargetKind::Ios => Self::Ios,
        }
    }
}

impl From<PortalTargetKindV1> for PortalTargetKind {
    fn from(kind: PortalTargetKindV1) -> Self {
        match kind {
            PortalTargetKindV1::Browser => Self::Browser,
            PortalTargetKindV1::Android => Self::Android,
            PortalTargetKindV1::Ios => Self::Ios,
        }
    }
}

fn project_path(value: String) -> Result<ProjectPath, String> {
    ProjectPath::new(value).map_err(|error| error.to_string())
}

fn color([red, green, blue, alpha]: [u8; 4]) -> CanvasColor {
    CanvasColor::rgba(red, green, blue, alpha)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DiffComparisonV1 {
    WorkingTreeAgainstHead,
}

impl From<DiffComparison> for DiffComparisonV1 {
    fn from(comparison: DiffComparison) -> Self {
        match comparison {
            DiffComparison::WorkingTreeAgainstHead => Self::WorkingTreeAgainstHead,
        }
    }
}

impl From<DiffComparisonV1> for DiffComparison {
    fn from(comparison: DiffComparisonV1) -> Self {
        match comparison {
            DiffComparisonV1::WorkingTreeAgainstHead => Self::WorkingTreeAgainstHead,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ShapeKindV1 {
    Rectangle,
    Ellipse,
}

impl From<ShapeKind> for ShapeKindV1 {
    fn from(kind: ShapeKind) -> Self {
        match kind {
            ShapeKind::Rectangle => Self::Rectangle,
            ShapeKind::Ellipse => Self::Ellipse,
        }
    }
}

impl From<ShapeKindV1> for ShapeKind {
    fn from(kind: ShapeKindV1) -> Self {
        match kind {
            ShapeKindV1::Rectangle => Self::Rectangle,
            ShapeKindV1::Ellipse => Self::Ellipse,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NormalizedPointV1 {
    x: f32,
    y: f32,
}

impl From<NormalizedPoint> for NormalizedPointV1 {
    fn from(point: NormalizedPoint) -> Self {
        Self {
            x: point.x(),
            y: point.y(),
        }
    }
}

impl NormalizedPointV1 {
    fn into_domain(self) -> Result<NormalizedPoint, String> {
        NormalizedPoint::new(self.x, self.y).map_err(|error| error.to_string())
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

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FloorsV1 {
    entries: Vec<(u64, FloorV1)>,
    node_floors: Vec<(u64, u64)>,
    active: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FloorV1 {
    name: String,
    directory: String,
    repository: String,
    branch: Option<String>,
    base_revision: String,
    base_branch: Option<String>,
    managed: bool,
    #[serde(default)]
    ownership_token: Option<String>,
    owner: Option<FloorOwnerV1>,
    dirty: bool,
    lifecycle: FloorLifecycleV1,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FloorOwnerV1 {
    Agent(u64),
    Task(u64),
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FloorLifecycleV1 {
    Available,
    Missing,
    Removed,
}

impl From<&crate::domain::Floors> for FloorsV1 {
    fn from(floors: &crate::domain::Floors) -> Self {
        use crate::domain::{FloorLifecycle, FloorOwner};
        Self {
            active: floors.active,
            node_floors: floors
                .node_floors
                .iter()
                .map(|(n, f)| (n.get(), *f))
                .collect(),
            entries: floors
                .entries
                .iter()
                .map(|(id, f)| {
                    (
                        *id,
                        FloorV1 {
                            name: f.name.as_str().to_owned(),
                            directory: f.directory.as_str().to_owned(),
                            repository: f.repository.as_str().to_owned(),
                            branch: f.branch.clone(),
                            base_revision: f.base_revision.clone(),
                            base_branch: f.base_branch.clone(),
                            managed: f.managed,
                            ownership_token: f.ownership_token.clone(),
                            dirty: f.dirty,
                            owner: f.owner.map(|o| match o {
                                FloorOwner::Agent(id) => FloorOwnerV1::Agent(id.get()),
                                FloorOwner::Task(id) => FloorOwnerV1::Task(id.get()),
                            }),
                            lifecycle: match f.lifecycle {
                                FloorLifecycle::Available => FloorLifecycleV1::Available,
                                FloorLifecycle::Missing => FloorLifecycleV1::Missing,
                                FloorLifecycle::Removed => FloorLifecycleV1::Removed,
                            },
                        },
                    )
                })
                .collect(),
        }
    }
}

impl FloorsV1 {
    fn into_domain(self) -> Result<crate::domain::Floors, String> {
        use crate::domain::{Floor, FloorLifecycle, FloorOwner, Floors};
        let entries = self
            .entries
            .into_iter()
            .map(|(id, f)| {
                Ok((
                    id,
                    Floor {
                        name: Name::new(f.name).map_err(|e| e.to_string())?,
                        directory: WorkspaceDirectory::new(f.directory)
                            .map_err(|e| e.to_string())?,
                        repository: WorkspaceDirectory::new(f.repository)
                            .map_err(|e| e.to_string())?,
                        branch: f.branch,
                        base_revision: f.base_revision,
                        base_branch: f.base_branch,
                        managed: f.managed,
                        ownership_token: f.ownership_token,
                        dirty: f.dirty,
                        owner: f.owner.map(|o| match o {
                            FloorOwnerV1::Agent(id) => FloorOwner::Agent(AgentId::new(id)),
                            FloorOwnerV1::Task(id) => FloorOwner::Task(TaskId::new(id)),
                        }),
                        lifecycle: match f.lifecycle {
                            FloorLifecycleV1::Available => FloorLifecycle::Available,
                            FloorLifecycleV1::Missing => FloorLifecycle::Missing,
                            FloorLifecycleV1::Removed => FloorLifecycle::Removed,
                        },
                    },
                ))
            })
            .collect::<Result<_, String>>()?;
        Ok(Floors {
            entries,
            node_floors: self
                .node_floors
                .into_iter()
                .map(|(n, f)| (NodeId::new(n), f))
                .collect(),
            active: self.active,
        })
    }
}

// Routine records. Every field the scheduler reads when it decides what to run
// next is stored, so a recovered run makes the same decisions the live one did.

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineV1 {
    id: u64,
    name: String,
    #[serde(default)]
    description: Option<String>,
    versions: Vec<RoutineVersionV1>,
    #[serde(default)]
    triggers: Vec<RoutineTriggerV1>,
}

impl From<&Routine> for RoutineV1 {
    fn from(routine: &Routine) -> Self {
        Self {
            id: routine.id().get(),
            name: routine.name().as_str().to_owned(),
            description: routine.description().map(|value| value.as_str().to_owned()),
            versions: routine
                .versions()
                .iter()
                .map(RoutineVersionV1::from)
                .collect(),
            triggers: routine.triggers().map(RoutineTriggerV1::from).collect(),
        }
    }
}

impl RoutineV1 {
    fn into_domain(self) -> Result<Routine, String> {
        let id = RoutineId::new(self.id);
        let versions = self
            .versions
            .into_iter()
            .map(RoutineVersionV1::into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        let triggers = self
            .triggers
            .into_iter()
            .map(RoutineTriggerV1::into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        Routine::restore(
            id,
            Name::new(self.name).map_err(|error| error.to_string())?,
            self.description
                .map(Content::new)
                .transpose()
                .map_err(|error| error.to_string())?,
            versions,
            triggers,
        )
        .map_err(|error| error.to_string())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineVersionV1 {
    id: u64,
    routine_id: u64,
    number: u32,
    inputs: Vec<RoutineInputV1>,
    steps: Vec<RoutineStepV1>,
    #[serde(default)]
    template: Option<String>,
    created_at: u64,
}

impl From<&RoutineVersion> for RoutineVersionV1 {
    fn from(version: &RoutineVersion) -> Self {
        Self {
            id: version.id().get(),
            routine_id: version.routine_id().get(),
            number: version.number(),
            inputs: version.inputs().iter().map(RoutineInputV1::from).collect(),
            steps: version.steps().iter().map(RoutineStepV1::from).collect(),
            template: version.template().map(|value| value.as_str().to_owned()),
            created_at: version.created_at().as_unix_millis(),
        }
    }
}

impl RoutineVersionV1 {
    fn into_domain(self) -> Result<RoutineVersion, String> {
        RoutineVersion::new(
            RoutineVersionId::new(self.id),
            RoutineId::new(self.routine_id),
            self.number,
            self.inputs
                .into_iter()
                .map(RoutineInputV1::into_domain)
                .collect::<Result<_, _>>()?,
            self.steps
                .into_iter()
                .map(RoutineStepV1::into_domain)
                .collect::<Result<_, _>>()?,
            self.template
                .map(Content::new)
                .transpose()
                .map_err(|error| error.to_string())?,
            Timestamp::from_unix_millis(self.created_at),
        )
        .map_err(|error| error.to_string())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineInputV1 {
    key: String,
    label: String,
    required: bool,
    #[serde(default)]
    default: Option<String>,
}

impl From<&RoutineInputDeclaration> for RoutineInputV1 {
    fn from(input: &RoutineInputDeclaration) -> Self {
        Self {
            key: input.key().as_str().to_owned(),
            label: input.label().as_str().to_owned(),
            required: input.required(),
            default: input.default().map(|value| value.as_str().to_owned()),
        }
    }
}

impl RoutineInputV1 {
    fn into_domain(self) -> Result<RoutineInputDeclaration, String> {
        Ok(RoutineInputDeclaration::new(
            RoutineInputKey::new(self.key).map_err(|error| error.to_string())?,
            Name::new(self.label).map_err(|error| error.to_string())?,
            self.required,
            self.default
                .map(RoutineValue::new)
                .transpose()
                .map_err(|error| error.to_string())?,
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineStepV1 {
    id: u64,
    name: String,
    agent_id: u64,
    prompt: String,
    #[serde(default)]
    depends_on: Vec<u64>,
    #[serde(default)]
    bindings: Vec<RoutineBindingV1>,
    #[serde(default)]
    outputs: Vec<String>,
    #[serde(default)]
    approval_required: bool,
    max_attempts: u32,
    checkout: RoutineCheckoutClaimV1,
    #[serde(default)]
    resources: Vec<String>,
}

impl From<&RoutineStep> for RoutineStepV1 {
    fn from(step: &RoutineStep) -> Self {
        Self {
            id: step.id().get(),
            name: step.name().as_str().to_owned(),
            agent_id: step.agent_id().get(),
            prompt: step.prompt().as_str().to_owned(),
            depends_on: step.depends_on().map(RoutineStepId::get).collect(),
            bindings: step
                .bindings()
                .map(|(name, source)| RoutineBindingV1 {
                    name: name.as_str().to_owned(),
                    source: source.into(),
                })
                .collect(),
            outputs: step.outputs().map(|key| key.as_str().to_owned()).collect(),
            approval_required: step.approval() == RoutineApproval::Required,
            max_attempts: step.retry().max_attempts(),
            checkout: step.claims().checkout().into(),
            resources: step
                .claims()
                .resources()
                .map(|key| key.as_str().to_owned())
                .collect(),
        }
    }
}

impl RoutineStepV1 {
    fn into_domain(self) -> Result<RoutineStep, String> {
        let bindings = self
            .bindings
            .into_iter()
            .map(RoutineBindingV1::into_domain)
            .collect::<Result<Vec<_>, String>>()?;
        let outputs = self
            .outputs
            .into_iter()
            .map(RoutineOutputKey::new)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let resources = self
            .resources
            .into_iter()
            .map(RoutineResourceKey::new)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        RoutineStep::new(
            RoutineStepId::new(self.id),
            Name::new(self.name).map_err(|error| error.to_string())?,
            AgentId::new(self.agent_id),
            Content::new(self.prompt).map_err(|error| error.to_string())?,
            self.depends_on.into_iter().map(RoutineStepId::new),
            bindings,
            outputs,
            if self.approval_required {
                RoutineApproval::Required
            } else {
                RoutineApproval::NotRequired
            },
            RoutineRetryPolicy::new(self.max_attempts).map_err(|error| error.to_string())?,
            RoutineStepClaims::new(self.checkout.into(), resources)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineBindingV1 {
    name: String,
    source: RoutineBindingSourceV1,
}

impl RoutineBindingV1 {
    fn into_domain(self) -> Result<(RoutineInputKey, RoutineBindingSource), String> {
        Ok((
            RoutineInputKey::new(self.name).map_err(|error| error.to_string())?,
            self.source.into_domain()?,
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum RoutineBindingSourceV1 {
    Input { key: String },
    StepOutput { step_id: u64, key: String },
    Literal { value: String },
}

impl From<&RoutineBindingSource> for RoutineBindingSourceV1 {
    fn from(source: &RoutineBindingSource) -> Self {
        match source {
            RoutineBindingSource::Input(key) => Self::Input {
                key: key.as_str().to_owned(),
            },
            RoutineBindingSource::StepOutput { step_id, key } => Self::StepOutput {
                step_id: step_id.get(),
                key: key.as_str().to_owned(),
            },
            RoutineBindingSource::Literal(value) => Self::Literal {
                value: value.as_str().to_owned(),
            },
        }
    }
}

impl RoutineBindingSourceV1 {
    fn into_domain(self) -> Result<RoutineBindingSource, String> {
        Ok(match self {
            Self::Input { key } => RoutineBindingSource::Input(
                RoutineInputKey::new(key).map_err(|error| error.to_string())?,
            ),
            Self::StepOutput { step_id, key } => RoutineBindingSource::StepOutput {
                step_id: RoutineStepId::new(step_id),
                key: RoutineOutputKey::new(key).map_err(|error| error.to_string())?,
            },
            Self::Literal { value } => RoutineBindingSource::Literal(
                RoutineValue::new(value).map_err(|error| error.to_string())?,
            ),
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum RoutineCheckoutClaimV1 {
    AgentDefault,
    Floor { floor: u64 },
}

impl From<RoutineCheckoutClaim> for RoutineCheckoutClaimV1 {
    fn from(claim: RoutineCheckoutClaim) -> Self {
        match claim {
            RoutineCheckoutClaim::AgentDefault => Self::AgentDefault,
            RoutineCheckoutClaim::Floor(floor) => Self::Floor { floor },
        }
    }
}

impl From<RoutineCheckoutClaimV1> for RoutineCheckoutClaim {
    fn from(claim: RoutineCheckoutClaimV1) -> Self {
        match claim {
            RoutineCheckoutClaimV1::AgentDefault => Self::AgentDefault,
            RoutineCheckoutClaimV1::Floor { floor } => Self::Floor(floor),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineTriggerV1 {
    id: u64,
    routine_id: u64,
    name: String,
    kind: RoutineTriggerKindV1,
    enabled: bool,
    #[serde(default)]
    inputs: Vec<RoutineValueEntryV1>,
    #[serde(default)]
    last_firing: Option<RoutineFiringV1>,
    #[serde(default)]
    skipped_occurrences: u32,
}

impl From<&RoutineTrigger> for RoutineTriggerV1 {
    fn from(trigger: &RoutineTrigger) -> Self {
        Self {
            id: trigger.id().get(),
            routine_id: trigger.routine_id().get(),
            name: trigger.name().as_str().to_owned(),
            kind: trigger.kind().into(),
            enabled: trigger.enabled(),
            inputs: trigger
                .inputs()
                .iter()
                .map(|(key, value)| RoutineValueEntryV1 {
                    key: key.as_str().to_owned(),
                    value: value.as_str().to_owned(),
                })
                .collect(),
            last_firing: trigger.last_firing().map(RoutineFiringV1::from),
            skipped_occurrences: trigger.skipped_occurrences(),
        }
    }
}

impl RoutineTriggerV1 {
    fn into_domain(self) -> Result<RoutineTrigger, String> {
        let inputs = self
            .inputs
            .into_iter()
            .map(|entry| {
                Ok((
                    RoutineInputKey::new(entry.key)
                        .map_err(|e: crate::domain::RoutineError| e.to_string())?,
                    RoutineValue::new(entry.value).map_err(|e| e.to_string())?,
                ))
            })
            .collect::<Result<_, String>>()?;
        let last_firing = self
            .last_firing
            .map(RoutineFiringV1::into_domain)
            .transpose()?;
        Ok(RoutineTrigger::new(
            RoutineTriggerId::new(self.id),
            RoutineId::new(self.routine_id),
            Name::new(self.name).map_err(|error| error.to_string())?,
            self.kind.into_domain()?,
            self.enabled,
            inputs,
        )
        .map_err(|error| error.to_string())?
        .with_history(last_firing, self.skipped_occurrences))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineValueEntryV1 {
    key: String,
    value: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum RoutineTriggerKindV1 {
    Manual,
    Filesystem {
        patterns: Vec<String>,
        debounce_ms: u64,
    },
    Git {
        refs: Vec<String>,
    },
    Schedule {
        cadence: RoutineCadenceV1,
        offset_minutes: i32,
        #[serde(default)]
        timezone_label: Option<String>,
        #[serde(default)]
        next_occurrence: Option<u64>,
    },
}

impl From<&RoutineTriggerKind> for RoutineTriggerKindV1 {
    fn from(kind: &RoutineTriggerKind) -> Self {
        match kind {
            RoutineTriggerKind::Manual => Self::Manual,
            RoutineTriggerKind::Filesystem {
                patterns,
                debounce_ms,
            } => Self::Filesystem {
                patterns: patterns.clone(),
                debounce_ms: *debounce_ms,
            },
            RoutineTriggerKind::Git { refs } => Self::Git { refs: refs.clone() },
            RoutineTriggerKind::Schedule(schedule) => Self::Schedule {
                cadence: schedule.cadence().into(),
                offset_minutes: schedule.offset_minutes(),
                timezone_label: schedule
                    .timezone_label()
                    .map(|label| label.as_str().to_owned()),
                next_occurrence: schedule.next_occurrence().map(Timestamp::as_unix_millis),
            },
        }
    }
}

impl RoutineTriggerKindV1 {
    fn into_domain(self) -> Result<RoutineTriggerKind, String> {
        Ok(match self {
            Self::Manual => RoutineTriggerKind::Manual,
            Self::Filesystem {
                patterns,
                debounce_ms,
            } => RoutineTriggerKind::Filesystem {
                patterns,
                debounce_ms,
            },
            Self::Git { refs } => RoutineTriggerKind::Git { refs },
            Self::Schedule {
                cadence,
                offset_minutes,
                timezone_label,
                next_occurrence,
            } => RoutineTriggerKind::Schedule(
                RoutineSchedule::new(
                    cadence.into(),
                    offset_minutes,
                    timezone_label
                        .map(Name::new)
                        .transpose()
                        .map_err(|error| error.to_string())?,
                    next_occurrence.map(Timestamp::from_unix_millis),
                )
                .map_err(|error| error.to_string())?,
            ),
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum RoutineCadenceV1 {
    Hourly {
        minute: u32,
    },
    Daily {
        hour: u32,
        minute: u32,
    },
    Weekly {
        weekday: u32,
        hour: u32,
        minute: u32,
    },
}

impl From<RoutineCadence> for RoutineCadenceV1 {
    fn from(cadence: RoutineCadence) -> Self {
        match cadence {
            RoutineCadence::Hourly { minute } => Self::Hourly { minute },
            RoutineCadence::Daily { hour, minute } => Self::Daily { hour, minute },
            RoutineCadence::Weekly {
                weekday,
                hour,
                minute,
            } => Self::Weekly {
                weekday,
                hour,
                minute,
            },
        }
    }
}

impl From<RoutineCadenceV1> for RoutineCadence {
    fn from(cadence: RoutineCadenceV1) -> Self {
        match cadence {
            RoutineCadenceV1::Hourly { minute } => Self::Hourly { minute },
            RoutineCadenceV1::Daily { hour, minute } => Self::Daily { hour, minute },
            RoutineCadenceV1::Weekly {
                weekday,
                hour,
                minute,
            } => Self::Weekly {
                weekday,
                hour,
                minute,
            },
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineFiringV1 {
    occurrence: String,
    fired_at: u64,
}

impl From<&RoutineTriggerFiring> for RoutineFiringV1 {
    fn from(firing: &RoutineTriggerFiring) -> Self {
        Self {
            occurrence: firing.occurrence().as_str().to_owned(),
            fired_at: firing.fired_at().as_unix_millis(),
        }
    }
}

impl RoutineFiringV1 {
    fn into_domain(self) -> Result<RoutineTriggerFiring, String> {
        Ok(RoutineTriggerFiring::new(
            RoutineOccurrenceKey::new(self.occurrence).map_err(|error| error.to_string())?,
            Timestamp::from_unix_millis(self.fired_at),
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineRunV1 {
    id: u64,
    #[serde(default)]
    trigger_id: Option<u64>,
    #[serde(default)]
    occurrence: Option<String>,
    pin: RoutineRunPinV1,
    state: RoutineRunStateV1,
    started_at: u64,
    #[serde(default)]
    finished_at: Option<u64>,
    steps: Vec<RoutineStepRunV1>,
}

impl From<&RoutineRun> for RoutineRunV1 {
    fn from(run: &RoutineRun) -> Self {
        Self {
            id: run.id().get(),
            trigger_id: run.trigger_id().map(RoutineTriggerId::get),
            occurrence: run
                .occurrence()
                .map(|occurrence| occurrence.as_str().to_owned()),
            pin: RoutineRunPinV1::from(run.pin()),
            state: run.state().into(),
            started_at: run.started_at().as_unix_millis(),
            finished_at: run.finished_at().map(Timestamp::as_unix_millis),
            steps: run.steps().map(RoutineStepRunV1::from).collect(),
        }
    }
}

impl RoutineRunV1 {
    fn into_domain(self) -> Result<RoutineRun, String> {
        RoutineRun::restore(
            RoutineRunId::new(self.id),
            self.trigger_id.map(RoutineTriggerId::new),
            self.occurrence
                .map(RoutineOccurrenceKey::new)
                .transpose()
                .map_err(|error| error.to_string())?,
            self.pin.into_domain()?,
            self.state.into(),
            Timestamp::from_unix_millis(self.started_at),
            self.finished_at.map(Timestamp::from_unix_millis),
            self.steps
                .into_iter()
                .map(RoutineStepRunV1::into_domain)
                .collect::<Result<_, _>>()?,
        )
        .map_err(|error| error.to_string())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineRunPinV1 {
    version: RoutineVersionV1,
    #[serde(default)]
    inputs: Vec<RoutineValueEntryV1>,
    agents: Vec<RoutineStepAgentV1>,
    checkouts: Vec<RoutineStepCheckoutV1>,
    #[serde(default)]
    revisions: Vec<RoutineRevisionV1>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineStepAgentV1 {
    step_id: u64,
    agent_id: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineStepCheckoutV1 {
    step_id: u64,
    checkout: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineRevisionV1 {
    checkout: String,
    revision: String,
}

impl From<&RoutineRunPin> for RoutineRunPinV1 {
    fn from(pin: &RoutineRunPin) -> Self {
        Self {
            version: RoutineVersionV1::from(pin.version()),
            inputs: pin
                .inputs()
                .iter()
                .map(|(key, value)| RoutineValueEntryV1 {
                    key: key.as_str().to_owned(),
                    value: value.as_str().to_owned(),
                })
                .collect(),
            agents: pin
                .agents()
                .map(|(step_id, agent_id)| RoutineStepAgentV1 {
                    step_id: step_id.get(),
                    agent_id: agent_id.get(),
                })
                .collect(),
            checkouts: pin
                .checkouts()
                .map(|(step_id, checkout)| RoutineStepCheckoutV1 {
                    step_id: step_id.get(),
                    checkout: checkout.as_str().to_owned(),
                })
                .collect(),
            revisions: pin
                .revisions()
                .iter()
                .map(|(checkout, revision)| RoutineRevisionV1 {
                    checkout: checkout.as_str().to_owned(),
                    revision: revision.clone(),
                })
                .collect(),
        }
    }
}

impl RoutineRunPinV1 {
    fn into_domain(self) -> Result<RoutineRunPin, String> {
        let inputs = self
            .inputs
            .into_iter()
            .map(|entry| {
                Ok((
                    RoutineInputKey::new(entry.key)
                        .map_err(|e: crate::domain::RoutineError| e.to_string())?,
                    RoutineValue::new(entry.value).map_err(|e| e.to_string())?,
                ))
            })
            .collect::<Result<_, String>>()?;
        let agents = self
            .agents
            .into_iter()
            .map(|entry| {
                (
                    RoutineStepId::new(entry.step_id),
                    AgentId::new(entry.agent_id),
                )
            })
            .collect();
        let checkouts = self
            .checkouts
            .into_iter()
            .map(|entry| {
                Ok((
                    RoutineStepId::new(entry.step_id),
                    routine_checkout(entry.checkout)?,
                ))
            })
            .collect::<Result<_, String>>()?;
        let revisions = self
            .revisions
            .into_iter()
            .map(|entry| Ok((routine_checkout(entry.checkout)?, entry.revision)))
            .collect::<Result<_, String>>()?;
        RoutineRunPin::new(
            self.version.into_domain()?,
            inputs,
            agents,
            checkouts,
            revisions,
        )
        .map_err(|error| error.to_string())
    }
}

fn routine_checkout(value: String) -> Result<RoutineCheckout, String> {
    WorkspaceDirectory::new(value)
        .map(RoutineCheckout::new)
        .map_err(|error| error.to_string())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineStepRunV1 {
    step_id: u64,
    state: RoutineStepStateV1,
    #[serde(default)]
    attempts: Vec<RoutineAttemptV1>,
    #[serde(default)]
    outputs: Vec<RoutineValueEntryV1>,
    #[serde(default)]
    approval: Option<RoutineApprovalV1>,
    #[serde(default)]
    interruption: Option<RoutineInterruptionV1>,
    #[serde(default)]
    failure: Option<String>,
}

impl From<&RoutineStepRun> for RoutineStepRunV1 {
    fn from(step: &RoutineStepRun) -> Self {
        Self {
            step_id: step.step_id().get(),
            state: step.state().into(),
            attempts: step.attempts().iter().map(RoutineAttemptV1::from).collect(),
            outputs: step
                .outputs()
                .iter()
                .map(|(key, value)| RoutineValueEntryV1 {
                    key: key.as_str().to_owned(),
                    value: value.as_str().to_owned(),
                })
                .collect(),
            approval: step.approval().map(RoutineApprovalV1::from),
            interruption: step.interruption().map(RoutineInterruptionV1::from),
            failure: step.failure().map(|value| value.as_str().to_owned()),
        }
    }
}

impl RoutineStepRunV1 {
    fn into_domain(self) -> Result<RoutineStepRun, String> {
        let outputs = self
            .outputs
            .into_iter()
            .map(|entry| {
                Ok((
                    RoutineOutputKey::new(entry.key)
                        .map_err(|e: crate::domain::RoutineError| e.to_string())?,
                    RoutineValue::new(entry.value).map_err(|e| e.to_string())?,
                ))
            })
            .collect::<Result<_, String>>()?;
        RoutineStepRun::restore(
            RoutineStepId::new(self.step_id),
            self.state.into(),
            self.attempts
                .into_iter()
                .map(RoutineAttemptV1::into_domain)
                .collect::<Result<_, _>>()?,
            outputs,
            self.approval
                .map(RoutineApprovalV1::into_domain)
                .transpose()?,
            self.interruption.map(RoutineInterruptionV1::into_domain),
            self.failure
                .map(Content::new)
                .transpose()
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineAttemptV1 {
    id: u64,
    ordinal: u32,
    task_id: u64,
    handoff_id: u64,
    agent_id: u64,
    checkout: String,
    #[serde(default)]
    resources: Vec<String>,
    started_at: u64,
    #[serde(default)]
    outcome: Option<RoutineAttemptOutcomeV1>,
    #[serde(default)]
    finished_at: Option<u64>,
}

impl From<&RoutineAttempt> for RoutineAttemptV1 {
    fn from(attempt: &RoutineAttempt) -> Self {
        Self {
            id: attempt.id().get(),
            ordinal: attempt.ordinal(),
            task_id: attempt.task_id().get(),
            handoff_id: attempt.handoff_id().get(),
            agent_id: attempt.agent_id().get(),
            checkout: attempt.checkout().as_str().to_owned(),
            resources: attempt
                .resources()
                .map(|key| key.as_str().to_owned())
                .collect(),
            started_at: attempt.started_at().as_unix_millis(),
            outcome: attempt.outcome().map(RoutineAttemptOutcomeV1::from),
            finished_at: attempt.finished_at().map(Timestamp::as_unix_millis),
        }
    }
}

impl RoutineAttemptV1 {
    fn into_domain(self) -> Result<RoutineAttempt, String> {
        let resources = self
            .resources
            .into_iter()
            .map(RoutineResourceKey::new)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let attempt = RoutineAttempt::new(
            RoutineAttemptId::new(self.id),
            self.ordinal,
            TaskId::new(self.task_id),
            HandoffId::new(self.handoff_id),
            AgentId::new(self.agent_id),
            routine_checkout(self.checkout)?,
            resources,
            Timestamp::from_unix_millis(self.started_at),
        )
        .map_err(|error| error.to_string())?;
        match (self.outcome, self.finished_at) {
            (Some(outcome), Some(finished_at)) => attempt
                .with_outcome(outcome.into(), Timestamp::from_unix_millis(finished_at))
                .map_err(|error| error.to_string()),
            (None, None) => Ok(attempt),
            _ => Err("routine attempt outcome and finish time must be stored together".to_owned()),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RoutineAttemptOutcomeV1 {
    Completed,
    Failed,
    Cancelled,
}

impl From<RoutineAttemptOutcome> for RoutineAttemptOutcomeV1 {
    fn from(outcome: RoutineAttemptOutcome) -> Self {
        match outcome {
            RoutineAttemptOutcome::Completed => Self::Completed,
            RoutineAttemptOutcome::Failed => Self::Failed,
            RoutineAttemptOutcome::Cancelled => Self::Cancelled,
        }
    }
}

impl From<RoutineAttemptOutcomeV1> for RoutineAttemptOutcome {
    fn from(outcome: RoutineAttemptOutcomeV1) -> Self {
        match outcome {
            RoutineAttemptOutcomeV1::Completed => Self::Completed,
            RoutineAttemptOutcomeV1::Failed => Self::Failed,
            RoutineAttemptOutcomeV1::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineApprovalV1 {
    approved: bool,
    decided_at: u64,
    #[serde(default)]
    note: Option<String>,
}

impl From<&RoutineApprovalRecord> for RoutineApprovalV1 {
    fn from(record: &RoutineApprovalRecord) -> Self {
        Self {
            approved: record.decision() == RoutineApprovalDecision::Approved,
            decided_at: record.decided_at().as_unix_millis(),
            note: record.note().map(|note| note.as_str().to_owned()),
        }
    }
}

impl RoutineApprovalV1 {
    fn into_domain(self) -> Result<RoutineApprovalRecord, String> {
        Ok(RoutineApprovalRecord::new(
            if self.approved {
                RoutineApprovalDecision::Approved
            } else {
                RoutineApprovalDecision::Rejected
            },
            Timestamp::from_unix_millis(self.decided_at),
            self.note
                .map(Content::new)
                .transpose()
                .map_err(|error| error.to_string())?,
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineInterruptionV1 {
    reason: RoutineInterruptionReasonV1,
    detected_at: u64,
}

impl From<&RoutineInterruption> for RoutineInterruptionV1 {
    fn from(interruption: &RoutineInterruption) -> Self {
        Self {
            reason: match interruption.reason() {
                RoutineInterruptionReason::DispatchOutcomeUnknown => {
                    RoutineInterruptionReasonV1::DispatchOutcomeUnknown
                }
                RoutineInterruptionReason::ShutdownUnconfirmed => {
                    RoutineInterruptionReasonV1::ShutdownUnconfirmed
                }
            },
            detected_at: interruption.detected_at().as_unix_millis(),
        }
    }
}

impl RoutineInterruptionV1 {
    fn into_domain(self) -> RoutineInterruption {
        RoutineInterruption::new(
            match self.reason {
                RoutineInterruptionReasonV1::DispatchOutcomeUnknown => {
                    RoutineInterruptionReason::DispatchOutcomeUnknown
                }
                RoutineInterruptionReasonV1::ShutdownUnconfirmed => {
                    RoutineInterruptionReason::ShutdownUnconfirmed
                }
            },
            Timestamp::from_unix_millis(self.detected_at),
        )
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RoutineInterruptionReasonV1 {
    DispatchOutcomeUnknown,
    ShutdownUnconfirmed,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RoutineRunStateV1 {
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl From<RoutineRunState> for RoutineRunStateV1 {
    fn from(state: RoutineRunState) -> Self {
        match state {
            RoutineRunState::Running => Self::Running,
            RoutineRunState::Cancelling => Self::Cancelling,
            RoutineRunState::Completed => Self::Completed,
            RoutineRunState::Failed => Self::Failed,
            RoutineRunState::Cancelled => Self::Cancelled,
            RoutineRunState::Interrupted => Self::Interrupted,
        }
    }
}

impl From<RoutineRunStateV1> for RoutineRunState {
    fn from(state: RoutineRunStateV1) -> Self {
        match state {
            RoutineRunStateV1::Running => Self::Running,
            RoutineRunStateV1::Cancelling => Self::Cancelling,
            RoutineRunStateV1::Completed => Self::Completed,
            RoutineRunStateV1::Failed => Self::Failed,
            RoutineRunStateV1::Cancelled => Self::Cancelled,
            RoutineRunStateV1::Interrupted => Self::Interrupted,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RoutineStepStateV1 {
    Pending,
    AwaitingApproval,
    Dispatched,
    Completed,
    Failed,
    Cancelled,
    Skipped,
    Interrupted,
}

impl From<RoutineStepState> for RoutineStepStateV1 {
    fn from(state: RoutineStepState) -> Self {
        match state {
            RoutineStepState::Pending => Self::Pending,
            RoutineStepState::AwaitingApproval => Self::AwaitingApproval,
            RoutineStepState::Dispatched => Self::Dispatched,
            RoutineStepState::Completed => Self::Completed,
            RoutineStepState::Failed => Self::Failed,
            RoutineStepState::Cancelled => Self::Cancelled,
            RoutineStepState::Skipped => Self::Skipped,
            RoutineStepState::Interrupted => Self::Interrupted,
        }
    }
}

impl From<RoutineStepStateV1> for RoutineStepState {
    fn from(state: RoutineStepStateV1) -> Self {
        match state {
            RoutineStepStateV1::Pending => Self::Pending,
            RoutineStepStateV1::AwaitingApproval => Self::AwaitingApproval,
            RoutineStepStateV1::Dispatched => Self::Dispatched,
            RoutineStepStateV1::Completed => Self::Completed,
            RoutineStepStateV1::Failed => Self::Failed,
            RoutineStepStateV1::Cancelled => Self::Cancelled,
            RoutineStepStateV1::Skipped => Self::Skipped,
            RoutineStepStateV1::Interrupted => Self::Interrupted,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum RoutineTransitionV1 {
    AwaitApproval {
        step_id: u64,
    },
    RecordApproval {
        step_id: u64,
        record: RoutineApprovalV1,
    },
    CompleteStep {
        step_id: u64,
        outputs: Vec<RoutineValueEntryV1>,
        finished_at: u64,
    },
    FailStep {
        step_id: u64,
        reason: String,
        finished_at: u64,
    },
    InterruptStep {
        step_id: u64,
        reason: RoutineInterruptionReasonV1,
        detected_at: u64,
    },
    ResolveInterruption {
        step_id: u64,
        finished_at: u64,
    },
    CancelStep {
        step_id: u64,
        #[serde(default)]
        reason: Option<String>,
        finished_at: u64,
    },
    RequestCancellation {
        at: u64,
    },
    Settle {
        at: u64,
    },
}

impl From<&RoutineTransition> for RoutineTransitionV1 {
    fn from(transition: &RoutineTransition) -> Self {
        match transition {
            RoutineTransition::AwaitApproval { step_id } => Self::AwaitApproval {
                step_id: step_id.get(),
            },
            RoutineTransition::RecordApproval { step_id, record } => Self::RecordApproval {
                step_id: step_id.get(),
                record: RoutineApprovalV1::from(record),
            },
            RoutineTransition::CompleteStep {
                step_id,
                outputs,
                finished_at,
            } => Self::CompleteStep {
                step_id: step_id.get(),
                outputs: outputs
                    .iter()
                    .map(|(key, value)| RoutineValueEntryV1 {
                        key: key.as_str().to_owned(),
                        value: value.as_str().to_owned(),
                    })
                    .collect(),
                finished_at: finished_at.as_unix_millis(),
            },
            RoutineTransition::FailStep {
                step_id,
                reason,
                finished_at,
            } => Self::FailStep {
                step_id: step_id.get(),
                reason: reason.as_str().to_owned(),
                finished_at: finished_at.as_unix_millis(),
            },
            RoutineTransition::InterruptStep {
                step_id,
                reason,
                detected_at,
            } => Self::InterruptStep {
                step_id: step_id.get(),
                reason: match reason {
                    RoutineInterruptionReason::DispatchOutcomeUnknown => {
                        RoutineInterruptionReasonV1::DispatchOutcomeUnknown
                    }
                    RoutineInterruptionReason::ShutdownUnconfirmed => {
                        RoutineInterruptionReasonV1::ShutdownUnconfirmed
                    }
                },
                detected_at: detected_at.as_unix_millis(),
            },
            RoutineTransition::ResolveInterruption {
                step_id,
                finished_at,
            } => Self::ResolveInterruption {
                step_id: step_id.get(),
                finished_at: finished_at.as_unix_millis(),
            },
            RoutineTransition::CancelStep {
                step_id,
                reason,
                finished_at,
            } => Self::CancelStep {
                step_id: step_id.get(),
                reason: reason.as_ref().map(|value| value.as_str().to_owned()),
                finished_at: finished_at.as_unix_millis(),
            },
            RoutineTransition::RequestCancellation { at } => Self::RequestCancellation {
                at: at.as_unix_millis(),
            },
            RoutineTransition::Settle { at } => Self::Settle {
                at: at.as_unix_millis(),
            },
        }
    }
}

impl RoutineTransitionV1 {
    fn into_domain(self) -> Result<RoutineTransition, String> {
        Ok(match self {
            Self::AwaitApproval { step_id } => RoutineTransition::AwaitApproval {
                step_id: RoutineStepId::new(step_id),
            },
            Self::RecordApproval { step_id, record } => RoutineTransition::RecordApproval {
                step_id: RoutineStepId::new(step_id),
                record: record.into_domain()?,
            },
            Self::CompleteStep {
                step_id,
                outputs,
                finished_at,
            } => RoutineTransition::CompleteStep {
                step_id: RoutineStepId::new(step_id),
                outputs: outputs
                    .into_iter()
                    .map(|entry| {
                        Ok((
                            RoutineOutputKey::new(entry.key)
                                .map_err(|e: crate::domain::RoutineError| e.to_string())?,
                            RoutineValue::new(entry.value).map_err(|e| e.to_string())?,
                        ))
                    })
                    .collect::<Result<_, String>>()?,
                finished_at: Timestamp::from_unix_millis(finished_at),
            },
            Self::FailStep {
                step_id,
                reason,
                finished_at,
            } => RoutineTransition::FailStep {
                step_id: RoutineStepId::new(step_id),
                reason: Content::new(reason).map_err(|error| error.to_string())?,
                finished_at: Timestamp::from_unix_millis(finished_at),
            },
            Self::InterruptStep {
                step_id,
                reason,
                detected_at,
            } => RoutineTransition::InterruptStep {
                step_id: RoutineStepId::new(step_id),
                reason: match reason {
                    RoutineInterruptionReasonV1::DispatchOutcomeUnknown => {
                        RoutineInterruptionReason::DispatchOutcomeUnknown
                    }
                    RoutineInterruptionReasonV1::ShutdownUnconfirmed => {
                        RoutineInterruptionReason::ShutdownUnconfirmed
                    }
                },
                detected_at: Timestamp::from_unix_millis(detected_at),
            },
            Self::ResolveInterruption {
                step_id,
                finished_at,
            } => RoutineTransition::ResolveInterruption {
                step_id: RoutineStepId::new(step_id),
                finished_at: Timestamp::from_unix_millis(finished_at),
            },
            Self::CancelStep {
                step_id,
                reason,
                finished_at,
            } => RoutineTransition::CancelStep {
                step_id: RoutineStepId::new(step_id),
                reason: reason
                    .map(Content::new)
                    .transpose()
                    .map_err(|error| error.to_string())?,
                finished_at: Timestamp::from_unix_millis(finished_at),
            },
            Self::RequestCancellation { at } => RoutineTransition::RequestCancellation {
                at: Timestamp::from_unix_millis(at),
            },
            Self::Settle { at } => RoutineTransition::Settle {
                at: Timestamp::from_unix_millis(at),
            },
        })
    }
}
