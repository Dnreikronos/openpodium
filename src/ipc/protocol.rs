use std::error::Error;
use std::fmt::{self, Debug, Display, Formatter};

use serde::{Deserialize, Serialize};

use super::{PROTOCOL_NAME, SUPPORTED_VERSIONS};

const MAX_ID_CHARS: usize = 128;
const MAX_TITLE_CHARS: usize = 255;
const MAX_BODY_CHARS: usize = 32_768;
const MAX_PORTAL_TARGET_CHARS: usize = 2_048;
const MAX_PORTAL_ELEMENT_CHARS: usize = 256;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MessageId(String);

impl MessageId {
    pub fn new(value: impl Into<String>) -> Result<Self, ProtocolValidationError> {
        let value = value.into();
        if value.is_empty() || value.chars().count() > MAX_ID_CHARS {
            return Err(ProtocolValidationError::InvalidIdentifier);
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(ProtocolValidationError::InvalidIdentifier);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Debug for MessageId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("MessageId").field(&self.0).finish()
    }
}

impl Display for MessageId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, formatter)
    }
}

impl TryFrom<String> for MessageId {
    type Error = ProtocolValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<MessageId> for String {
    fn from(value: MessageId) -> Self {
        value.0
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    pub workspace_id: u64,
    pub agent_id: u64,
    pub token: String,
}

impl Debug for Credentials {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("workspace_id", &self.workspace_id)
            .field("agent_id", &self.agent_id)
            .field("token", &"[redacted]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolRequest {
    pub protocol: String,
    pub supported_versions: Vec<u16>,
    pub request_id: MessageId,
    pub credentials: Credentials,
    pub command: ProtocolCommand,
}

impl ProtocolRequest {
    pub fn new(request_id: MessageId, credentials: Credentials, command: ProtocolCommand) -> Self {
        Self {
            protocol: PROTOCOL_NAME.to_owned(),
            supported_versions: SUPPORTED_VERSIONS.to_vec(),
            request_id,
            credentials,
            command,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProtocolCommand {
    ListAgents,
    ListPortals,
    InspectPortal {
        portal_id: u64,
    },
    ObservePortal {
        portal_id: u64,
    },
    RequestPortalAction {
        action_id: MessageId,
        portal_id: u64,
        action: PortalActionRequest,
    },
    GetPortalResult {
        action_id: MessageId,
    },
    SendTask {
        message_id: MessageId,
        recipient_agent_id: u64,
        title: String,
        body: String,
    },
    SendHandoff {
        message_id: MessageId,
        recipient_agent_id: u64,
        kind: HandoffKind,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        body: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_message_id: Option<MessageId>,
        #[serde(skip_serializing_if = "Option::is_none")]
        response_timeout_ms: Option<u64>,
    },
    ReportProgress {
        message_id: MessageId,
        task_message_id: MessageId,
        body: String,
    },
    Respond {
        message_id: MessageId,
        task_message_id: MessageId,
        status: ResponseStatus,
        body: String,
    },
    ReportHandoffProgress {
        message_id: MessageId,
        handoff_message_id: MessageId,
        body: String,
    },
    RespondToHandoff {
        message_id: MessageId,
        handoff_message_id: MessageId,
        status: ResponseStatus,
        body: String,
    },
    CancelHandoff {
        message_id: MessageId,
        handoff_message_id: MessageId,
        reason: String,
    },
}

impl ProtocolCommand {
    pub fn message_id(&self) -> Option<&MessageId> {
        match self {
            Self::ListAgents => None,
            Self::ListPortals | Self::InspectPortal { .. } | Self::ObservePortal { .. } => None,
            Self::RequestPortalAction { action_id, .. } => Some(action_id),
            Self::GetPortalResult { .. } => None,
            Self::SendTask { message_id, .. }
            | Self::SendHandoff { message_id, .. }
            | Self::ReportProgress { message_id, .. }
            | Self::Respond { message_id, .. }
            | Self::ReportHandoffProgress { message_id, .. }
            | Self::RespondToHandoff { message_id, .. }
            | Self::CancelHandoff { message_id, .. } => Some(message_id),
        }
    }

    pub const fn minimum_version(&self) -> u16 {
        match self {
            Self::ListAgents
            | Self::SendTask { .. }
            | Self::ReportProgress { .. }
            | Self::Respond { .. } => 1,
            Self::SendHandoff { .. }
            | Self::ReportHandoffProgress { .. }
            | Self::RespondToHandoff { .. }
            | Self::CancelHandoff { .. } => 2,
            Self::ListPortals
            | Self::InspectPortal { .. }
            | Self::ObservePortal { .. }
            | Self::RequestPortalAction { .. }
            | Self::GetPortalResult { .. } => 3,
        }
    }

    pub const fn is_portal(&self) -> bool {
        matches!(
            self,
            Self::ListPortals
                | Self::InspectPortal { .. }
                | Self::ObservePortal { .. }
                | Self::RequestPortalAction { .. }
                | Self::GetPortalResult { .. }
        )
    }

    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        match self {
            Self::ListAgents => Ok(()),
            Self::ListPortals => Ok(()),
            Self::InspectPortal { portal_id } | Self::ObservePortal { portal_id } => {
                validate_portal_id(*portal_id)
            }
            Self::RequestPortalAction {
                portal_id, action, ..
            } => {
                validate_portal_id(*portal_id)?;
                action.validate()
            }
            Self::GetPortalResult { .. } => Ok(()),
            Self::SendTask {
                recipient_agent_id,
                title,
                body,
                ..
            } => {
                if *recipient_agent_id == 0 {
                    return Err(ProtocolValidationError::InvalidAgentId);
                }
                validate_text(title, MAX_TITLE_CHARS, "task title")?;
                validate_text(body, MAX_BODY_CHARS, "task body")
            }
            Self::SendHandoff {
                recipient_agent_id,
                kind,
                title,
                body,
                response_timeout_ms,
                ..
            } => {
                if *recipient_agent_id == 0 {
                    return Err(ProtocolValidationError::InvalidAgentId);
                }
                match (kind, title) {
                    (HandoffKind::Task, Some(title)) => {
                        validate_text(title, MAX_TITLE_CHARS, "task title")?;
                    }
                    (HandoffKind::Task, None) => {
                        return Err(ProtocolValidationError::MissingTaskTitle);
                    }
                    (HandoffKind::Question, Some(_)) => {
                        return Err(ProtocolValidationError::UnexpectedQuestionTitle);
                    }
                    (HandoffKind::Question, None) => {}
                }
                if response_timeout_ms == &Some(0) {
                    return Err(ProtocolValidationError::InvalidResponseTimeout);
                }
                validate_text(body, MAX_BODY_CHARS, "handoff body")
            }
            Self::ReportProgress { body, .. }
            | Self::Respond { body, .. }
            | Self::ReportHandoffProgress { body, .. }
            | Self::RespondToHandoff { body, .. } => {
                validate_text(body, MAX_BODY_CHARS, "message body")
            }
            Self::CancelHandoff { reason, .. } => {
                validate_text(reason, MAX_BODY_CHARS, "cancellation reason")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PortalActionRequest {
    Click {
        element_id: String,
        observation_revision: u64,
    },
    TypeText {
        element_id: String,
        observation_revision: u64,
        text: String,
    },
    Scroll {
        #[serde(skip_serializing_if = "Option::is_none")]
        element_id: Option<String>,
        observation_revision: u64,
        delta_x: i32,
        delta_y: i32,
    },
    Navigate {
        target: String,
    },
}

impl PortalActionRequest {
    fn validate(&self) -> Result<(), ProtocolValidationError> {
        match self {
            Self::Click {
                element_id,
                observation_revision,
            } => validate_element(element_id, *observation_revision),
            Self::TypeText {
                element_id,
                observation_revision,
                text,
            } => {
                validate_element(element_id, *observation_revision)?;
                validate_text(text, MAX_BODY_CHARS, "portal input")
            }
            Self::Scroll {
                element_id,
                observation_revision,
                delta_x,
                delta_y,
            } => {
                if let Some(element_id) = element_id {
                    validate_element(element_id, *observation_revision)?;
                } else if *observation_revision == 0 {
                    return Err(ProtocolValidationError::InvalidObservationRevision);
                }
                if *delta_x == 0 && *delta_y == 0 {
                    return Err(ProtocolValidationError::EmptyPortalScroll);
                }
                Ok(())
            }
            Self::Navigate { target } => {
                validate_text(target, MAX_PORTAL_TARGET_CHARS, "portal navigation target")
            }
        }
    }
}

fn validate_portal_id(portal_id: u64) -> Result<(), ProtocolValidationError> {
    portal_id_is_positive(portal_id)
        .then_some(())
        .ok_or(ProtocolValidationError::InvalidPortalId)
}

fn portal_id_is_positive(portal_id: u64) -> bool {
    portal_id > 0
}

fn validate_element(
    element_id: &str,
    observation_revision: u64,
) -> Result<(), ProtocolValidationError> {
    validate_text(
        element_id,
        MAX_PORTAL_ELEMENT_CHARS,
        "portal element reference",
    )?;
    if observation_revision == 0 {
        return Err(ProtocolValidationError::InvalidObservationRevision);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffKind {
    Task,
    Question,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    Completed,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCapabilities {
    pub accepts_tasks: bool,
    #[serde(default)]
    pub accepts_questions: bool,
    pub reports_progress: bool,
    pub responds: bool,
    #[serde(default)]
    pub supports_cancellation: bool,
}

impl AgentCapabilities {
    pub const CONNECTED: Self = Self {
        accepts_tasks: true,
        accepts_questions: true,
        reports_progress: true,
        responds: true,
        supports_cancellation: true,
    };

    pub const UNAVAILABLE: Self = Self {
        accepts_tasks: false,
        accepts_questions: false,
        reports_progress: false,
        responds: false,
        supports_cancellation: false,
    };
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDescriptor {
    pub id: u64,
    pub name: String,
    pub program: String,
    pub state: String,
    pub is_self: bool,
    pub capabilities: AgentCapabilities,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortalTargetKind {
    Browser,
    Android,
    Ios,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalDescriptor {
    pub id: u64,
    pub target_kind: PortalTargetKind,
    pub target: String,
    pub state: String,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalCapability {
    pub operation: String,
    pub status: PortalCapabilityStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PortalCapabilityStatus {
    Supported,
    Unavailable { reason: String },
    PermissionRequired { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalObservation {
    pub portal_id: u64,
    pub revision: u64,
    pub accessibility: Option<String>,
    pub frame_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortalActionState {
    Queued,
    AwaitingApproval,
    Dispatched,
    Completed,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PortalPolicyOutcome {
    Allowed,
    ApprovalRequired { approval_id: u64, reason: String },
    Denied { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalActionReceipt {
    pub action_id: MessageId,
    pub portal_id: u64,
    pub state: PortalActionState,
    pub policy: PortalPolicyOutcome,
    pub created_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatched_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolResponse {
    pub protocol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<MessageId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ProtocolResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

impl ProtocolResponse {
    pub fn success(version: u16, request_id: MessageId, result: ProtocolResult) -> Self {
        Self {
            protocol: PROTOCOL_NAME.to_owned(),
            version: Some(version),
            request_id: Some(request_id),
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(
        version: Option<u16>,
        request_id: Option<MessageId>,
        error: ProtocolError,
    ) -> Self {
        Self {
            protocol: PROTOCOL_NAME.to_owned(),
            version,
            request_id,
            result: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProtocolResult {
    Agents {
        agents: Vec<AgentDescriptor>,
    },
    Portals {
        portals: Vec<PortalDescriptor>,
    },
    PortalCapabilities {
        portal_id: u64,
        capabilities: Vec<PortalCapability>,
    },
    PortalObservation(PortalObservation),
    PortalReceipt(PortalActionReceipt),
    Accepted {
        message_id: MessageId,
        duplicate: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_versions: Option<Vec<u16>>,
}

impl ProtocolError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            supported_versions: None,
        }
    }

    pub fn incompatible(message: impl Into<String>, supported_versions: Vec<u16>) -> Self {
        Self {
            code: ErrorCode::IncompatibleProtocol,
            message: message.into(),
            supported_versions: Some(supported_versions),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    MalformedRequest,
    FrameTooLarge,
    IncompatibleProtocol,
    Unauthorized,
    InvalidRequest,
    AgentNotVisible,
    IdempotencyConflict,
    ServiceUnavailable,
    PortalUnavailable,
    PortalPolicyDenied,
    PortalApprovalRequired,
    StalePortalObservation,
    UnknownPortalAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolValidationError {
    InvalidIdentifier,
    InvalidAgentId,
    InvalidPortalId,
    InvalidObservationRevision,
    EmptyPortalScroll,
    MissingTaskTitle,
    UnexpectedQuestionTitle,
    InvalidResponseTimeout,
    EmptyText {
        field: &'static str,
    },
    TextTooLong {
        field: &'static str,
        max_chars: usize,
    },
    ControlCharacter {
        field: &'static str,
    },
}

impl Display for ProtocolValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier => formatter.write_str(
                "identifier must contain 1 to 128 ASCII letters, digits, dots, dashes, or underscores",
            ),
            Self::InvalidAgentId => formatter.write_str("agent ID must be greater than zero"),
            Self::InvalidPortalId => formatter.write_str("portal ID must be greater than zero"),
            Self::InvalidObservationRevision => {
                formatter.write_str("portal observation revision must be greater than zero")
            }
            Self::EmptyPortalScroll => {
                formatter.write_str("portal scroll must move along at least one axis")
            }
            Self::MissingTaskTitle => formatter.write_str("task handoffs require a title"),
            Self::UnexpectedQuestionTitle => {
                formatter.write_str("question handoffs cannot include a title")
            }
            Self::InvalidResponseTimeout => {
                formatter.write_str("response timeout must be greater than zero")
            }
            Self::EmptyText { field } => write!(formatter, "{field} cannot be empty"),
            Self::TextTooLong { field, max_chars } => {
                write!(formatter, "{field} cannot exceed {max_chars} characters")
            }
            Self::ControlCharacter { field } => {
                write!(formatter, "{field} contains an unsupported control character")
            }
        }
    }
}

impl Error for ProtocolValidationError {}

fn validate_text(
    value: &str,
    max_chars: usize,
    field: &'static str,
) -> Result<(), ProtocolValidationError> {
    if value.trim().is_empty() {
        return Err(ProtocolValidationError::EmptyText { field });
    }
    if value.chars().count() > max_chars {
        return Err(ProtocolValidationError::TextTooLong { field, max_chars });
    }
    if value.contains('\0') {
        return Err(ProtocolValidationError::ControlCharacter { field });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credentials() -> Credentials {
        Credentials {
            workspace_id: 4,
            agent_id: 7,
            token: "secret-token".to_owned(),
        }
    }

    #[test]
    fn request_round_trip_preserves_tagged_command() {
        let request = ProtocolRequest::new(
            MessageId::new("request-1").unwrap(),
            credentials(),
            ProtocolCommand::SendTask {
                message_id: MessageId::new("message-1").unwrap(),
                recipient_agent_id: 8,
                title: "Review".to_owned(),
                body: "Review the IPC protocol".to_owned(),
            },
        );

        let encoded = serde_json::to_string(&request).unwrap();
        let decoded: ProtocolRequest = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, request);
        assert!(encoded.contains("\"type\":\"send_task\""));
        assert!(!format!("{request:?}").contains("secret-token"));
    }

    #[test]
    fn identifiers_and_message_fields_are_bounded() {
        assert!(MessageId::new("task.01_retry-2").is_ok());
        assert!(MessageId::new("bad id").is_err());
        assert!(MessageId::new("x".repeat(MAX_ID_CHARS + 1)).is_err());

        let empty = ProtocolCommand::ReportProgress {
            message_id: MessageId::new("progress-1").unwrap(),
            task_message_id: MessageId::new("task-1").unwrap(),
            body: "   ".to_owned(),
        };
        assert_eq!(
            empty.validate(),
            Err(ProtocolValidationError::EmptyText {
                field: "message body"
            })
        );
    }

    #[test]
    fn version_two_handoffs_validate_kind_specific_fields() {
        let question = ProtocolCommand::SendHandoff {
            message_id: MessageId::new("question-1").unwrap(),
            recipient_agent_id: 8,
            kind: HandoffKind::Question,
            title: None,
            body: "Which API should I use?".to_owned(),
            parent_message_id: Some(MessageId::new("task-1").unwrap()),
            response_timeout_ms: Some(30_000),
        };
        assert_eq!(question.minimum_version(), 2);
        assert_eq!(question.validate(), Ok(()));

        let missing_title = ProtocolCommand::SendHandoff {
            message_id: MessageId::new("task-2").unwrap(),
            recipient_agent_id: 8,
            kind: HandoffKind::Task,
            title: None,
            body: "Do work".to_owned(),
            parent_message_id: None,
            response_timeout_ms: None,
        };
        assert_eq!(
            missing_title.validate(),
            Err(ProtocolValidationError::MissingTaskTitle)
        );
    }

    #[test]
    fn version_three_portal_actions_require_current_observations() {
        let action = ProtocolCommand::RequestPortalAction {
            action_id: MessageId::new("action-1").unwrap(),
            portal_id: 9,
            action: PortalActionRequest::Click {
                element_id: "submit-button".to_owned(),
                observation_revision: 4,
            },
        };
        assert_eq!(action.minimum_version(), 3);
        assert_eq!(action.validate(), Ok(()));

        let encoded = serde_json::to_string(&action).unwrap();
        assert!(encoded.contains("request_portal_action"));
        assert!(encoded.contains("click"));

        let stale = ProtocolCommand::RequestPortalAction {
            action_id: MessageId::new("action-2").unwrap(),
            portal_id: 9,
            action: PortalActionRequest::Click {
                element_id: "submit-button".to_owned(),
                observation_revision: 0,
            },
        };
        assert_eq!(
            stale.validate(),
            Err(ProtocolValidationError::InvalidObservationRevision)
        );
    }

    #[test]
    fn portal_read_commands_have_no_message_id_for_agent_routing() {
        for command in [
            ProtocolCommand::ListPortals,
            ProtocolCommand::InspectPortal { portal_id: 9 },
            ProtocolCommand::ObservePortal { portal_id: 9 },
            ProtocolCommand::GetPortalResult {
                action_id: MessageId::new("action-1").unwrap(),
            },
        ] {
            assert!(command.is_portal());
            assert_eq!(command.message_id(), None);
            assert_eq!(command.minimum_version(), 3);
        }
    }

    #[test]
    fn response_has_exactly_one_outcome() {
        let success = ProtocolResponse::success(
            2,
            MessageId::new("request-1").unwrap(),
            ProtocolResult::Agents { agents: Vec::new() },
        );
        assert!(success.result.is_some());
        assert!(success.error.is_none());

        let failure = ProtocolResponse::failure(
            Some(2),
            None,
            ProtocolError::new(ErrorCode::MalformedRequest, "invalid JSON"),
        );
        assert!(failure.result.is_none());
        assert!(failure.error.is_some());
    }
}
