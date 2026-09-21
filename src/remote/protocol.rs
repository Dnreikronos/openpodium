use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

macro_rules! byte_id {
    ($name:ident, $bytes:expr) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        pub struct $name(pub(crate) [u8; $bytes]);

        impl $name {
            pub const fn from_bytes(bytes: [u8; $bytes]) -> Self {
                Self(bytes)
            }

            pub const fn as_bytes(&self) -> &[u8; $bytes] {
                &self.0
            }
        }
    };
}

byte_id!(DeviceId, 16);
byte_id!(PairingId, 16);
byte_id!(SessionId, 16);
byte_id!(CommandId, 16);
byte_id!(EventId, 16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    ObserveTasks,
    ObserveChat,
    ObserveNotifications,
    ObserveCanvas,
    Prompt,
    Approve,
    Cancel,
    Resume,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerGrant {
    workspaces: BTreeSet<u64>,
    capabilities: BTreeSet<Capability>,
}

impl PeerGrant {
    pub fn new(
        workspaces: impl IntoIterator<Item = u64>,
        capabilities: impl IntoIterator<Item = Capability>,
    ) -> Self {
        Self {
            workspaces: workspaces.into_iter().collect(),
            capabilities: capabilities.into_iter().collect(),
        }
    }

    pub fn allows(&self, workspace_id: u64, capability: Capability) -> bool {
        self.workspaces.contains(&workspace_id) && self.capabilities.contains(&capability)
    }

    pub fn workspaces(&self) -> &BTreeSet<u64> {
        &self.workspaces
    }

    pub fn capabilities(&self) -> &BTreeSet<Capability> {
        &self.capabilities
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDelta {
    pub event_id: EventId,
    pub workspace_id: u64,
    pub task_id: u64,
    pub title: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatDelta {
    pub event_id: EventId,
    pub workspace_id: u64,
    pub thread_id: u64,
    pub message_id: u64,
    pub author: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationDelta {
    pub event_id: EventId,
    pub workspace_id: u64,
    pub class: String,
    pub summary: String,
    pub evidence_event_ids: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanvasDelta {
    pub event_id: EventId,
    pub workspace_id: u64,
    pub revision: u64,
    pub selected_node_ids: Vec<u64>,
    pub encoded_state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "delta", rename_all = "snake_case")]
pub enum StreamDelta {
    Task(TaskDelta),
    Chat(ChatDelta),
    Notification(NotificationDelta),
    Canvas(CanvasDelta),
}

impl StreamDelta {
    pub const fn workspace_id(&self) -> u64 {
        match self {
            Self::Task(delta) => delta.workspace_id,
            Self::Chat(delta) => delta.workspace_id,
            Self::Notification(delta) => delta.workspace_id,
            Self::Canvas(delta) => delta.workspace_id,
        }
    }

    pub const fn required_capability(&self) -> Capability {
        match self {
            Self::Task(_) => Capability::ObserveTasks,
            Self::Chat(_) => Capability::ObserveChat,
            Self::Notification(_) => Capability::ObserveNotifications,
            Self::Canvas(_) => Capability::ObserveCanvas,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "arguments", rename_all = "snake_case")]
pub enum SteeringAction {
    Prompt { agent_id: u64, body: String },
    Approve { approval_id: String },
    Cancel { task_id: u64 },
    Resume { task_id: u64 },
}

impl SteeringAction {
    pub const fn required_capability(&self) -> Capability {
        match self {
            Self::Prompt { .. } => Capability::Prompt,
            Self::Approve { .. } => Capability::Approve,
            Self::Cancel { .. } => Capability::Cancel,
            Self::Resume { .. } => Capability::Resume,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SteeringCommand {
    pub id: CommandId,
    pub workspace_id: u64,
    pub action: SteeringAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "detail", rename_all = "snake_case")]
pub enum CommandOutcome {
    Applied,
    Rejected(String),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandReceipt {
    pub command_id: CommandId,
    pub outcome: CommandOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticState {
    Connecting,
    Connected,
    Reconnecting,
    Offline,
    Revoked,
    ProtocolMismatch,
    AuthorizationFailure,
    CryptographicFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionDiagnostic {
    pub state: DiagnosticState,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum RemotePayload {
    Event(StreamDelta),
    Command(SteeringCommand),
    Receipt(CommandReceipt),
    Diagnostic(ConnectionDiagnostic),
}

impl RemotePayload {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        const MAX_TEXT_CHARS: usize = 64 * 1024;
        const MAX_SHORT_CHARS: usize = 512;
        const MAX_SELECTED_NODES: usize = 256;
        const MAX_EVIDENCE: usize = 256;

        let within = |value: &str, max: usize| value.chars().count() <= max;
        match self {
            Self::Event(StreamDelta::Task(delta)) => {
                if within(&delta.title, MAX_SHORT_CHARS) && within(&delta.state, MAX_SHORT_CHARS) {
                    Ok(())
                } else {
                    Err("task delta text exceeds protocol limits")
                }
            }
            Self::Event(StreamDelta::Chat(delta)) => {
                if within(&delta.author, MAX_SHORT_CHARS) && within(&delta.body, MAX_TEXT_CHARS) {
                    Ok(())
                } else {
                    Err("chat delta text exceeds protocol limits")
                }
            }
            Self::Event(StreamDelta::Notification(delta)) => {
                if within(&delta.class, MAX_SHORT_CHARS)
                    && within(&delta.summary, MAX_TEXT_CHARS)
                    && delta.evidence_event_ids.len() <= MAX_EVIDENCE
                {
                    Ok(())
                } else {
                    Err("notification delta exceeds protocol limits")
                }
            }
            Self::Event(StreamDelta::Canvas(delta)) => {
                if delta.selected_node_ids.len() <= MAX_SELECTED_NODES
                    && within(&delta.encoded_state, MAX_TEXT_CHARS)
                {
                    Ok(())
                } else {
                    Err("canvas delta exceeds protocol limits")
                }
            }
            Self::Command(SteeringCommand { action, .. }) => match action {
                SteeringAction::Prompt { body, .. } if !within(body, MAX_TEXT_CHARS) => {
                    Err("prompt exceeds protocol limits")
                }
                SteeringAction::Approve { approval_id }
                    if !within(approval_id, MAX_SHORT_CHARS) =>
                {
                    Err("approval identifier exceeds protocol limits")
                }
                _ => Ok(()),
            },
            Self::Receipt(receipt) => match &receipt.outcome {
                CommandOutcome::Rejected(detail) | CommandOutcome::Failed(detail)
                    if !within(detail, MAX_TEXT_CHARS) =>
                {
                    Err("command receipt exceeds protocol limits")
                }
                _ => Ok(()),
            },
            Self::Diagnostic(diagnostic) if !within(&diagnostic.detail, MAX_SHORT_CHARS) => {
                Err("diagnostic exceeds protocol limits")
            }
            Self::Diagnostic(_) => Ok(()),
        }
    }
}
