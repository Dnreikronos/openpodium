use std::error::Error;
use std::fmt::{self, Debug, Display, Formatter};

use serde::{Deserialize, Serialize};

use super::{PROTOCOL_NAME, PROTOCOL_VERSION};

const MAX_ID_CHARS: usize = 128;
const MAX_TITLE_CHARS: usize = 255;
const MAX_BODY_CHARS: usize = 32_768;

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
            supported_versions: vec![PROTOCOL_VERSION],
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
    SendTask {
        message_id: MessageId,
        recipient_agent_id: u64,
        title: String,
        body: String,
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
}

impl ProtocolCommand {
    pub fn message_id(&self) -> Option<&MessageId> {
        match self {
            Self::ListAgents => None,
            Self::SendTask { message_id, .. }
            | Self::ReportProgress { message_id, .. }
            | Self::Respond { message_id, .. } => Some(message_id),
        }
    }

    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        match self {
            Self::ListAgents => Ok(()),
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
            Self::ReportProgress { body, .. } | Self::Respond { body, .. } => {
                validate_text(body, MAX_BODY_CHARS, "message body")
            }
        }
    }
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
    pub reports_progress: bool,
    pub responds: bool,
}

impl AgentCapabilities {
    pub const CONNECTED: Self = Self {
        accepts_tasks: true,
        reports_progress: true,
        responds: true,
    };

    pub const UNAVAILABLE: Self = Self {
        accepts_tasks: false,
        reports_progress: false,
        responds: false,
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
    pub fn success(request_id: MessageId, result: ProtocolResult) -> Self {
        Self {
            protocol: PROTOCOL_NAME.to_owned(),
            version: Some(PROTOCOL_VERSION),
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolValidationError {
    InvalidIdentifier,
    InvalidAgentId,
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
    fn response_has_exactly_one_outcome() {
        let success = ProtocolResponse::success(
            MessageId::new("request-1").unwrap(),
            ProtocolResult::Agents { agents: Vec::new() },
        );
        assert!(success.result.is_some());
        assert!(success.error.is_none());

        let failure = ProtocolResponse::failure(
            Some(PROTOCOL_VERSION),
            None,
            ProtocolError::new(ErrorCode::MalformedRequest, "invalid JSON"),
        );
        assert!(failure.result.is_none());
        assert!(failure.error.is_some());
    }
}
