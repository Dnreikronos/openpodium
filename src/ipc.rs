//! Authenticated, workspace-scoped communication with launched agents.

mod auth;
mod cli;
mod client;
mod portal;
mod protocol;
mod server;
mod store;

pub use auth::{AuthenticationError, CapabilityIssuer};
pub use cli::run_cli;
pub use client::{
    AGENT_ID_ENV, AVAILABLE_ENV, CLI_ENV, ClientError, ConnectionConfig, ENDPOINT_ENV, IpcClient,
    TOKEN_ENV, VERSIONS_ENV, WORKSPACE_ID_ENV,
};
pub use portal::{
    PortalDispatcher, PortalInspection, PortalObservationResult, PortalScope, PortalServiceError,
};
pub use protocol::{
    AgentCapabilities, AgentDescriptor, Credentials, ErrorCode, HandoffKind, MessageId,
    PortalActionReceipt, PortalActionRequest, PortalActionState, PortalCapability,
    PortalCapabilityStatus, PortalDescriptor, PortalObservation, PortalPolicyOutcome,
    PortalTargetKind, ProtocolCommand, ProtocolError, ProtocolRequest, ProtocolResponse,
    ProtocolResult, ProtocolValidationError, ResponseStatus,
};
pub use server::{
    AcceptedMessage, AgentRegistration, ConnectionInfo, IpcService, PortalControl, ServiceError,
};

pub const PROTOCOL_NAME: &str = "openpodium-ipc";
pub const PROTOCOL_VERSION: u16 = 3;
pub const PREVIOUS_PROTOCOL_VERSION: u16 = 2;
pub const LEGACY_PROTOCOL_VERSION: u16 = 1;
pub const SUPPORTED_VERSIONS: &[u16] = &[
    PROTOCOL_VERSION,
    PREVIOUS_PROTOCOL_VERSION,
    LEGACY_PROTOCOL_VERSION,
];
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const SECRET_FILE_NAME: &str = "ipc-secret";
pub const MESSAGE_STORE_FILE_NAME: &str = "ipc-messages.sqlite";
