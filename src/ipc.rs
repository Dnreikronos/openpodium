//! Authenticated, workspace-scoped communication with launched agents.

mod auth;
mod cli;
mod client;
mod protocol;
mod server;

pub use auth::{AuthenticationError, CapabilityIssuer};
pub use cli::run_cli;
pub use client::{
    AGENT_ID_ENV, AVAILABLE_ENV, CLI_ENV, ClientError, ConnectionConfig, ENDPOINT_ENV, IpcClient,
    TOKEN_ENV, VERSIONS_ENV, WORKSPACE_ID_ENV,
};
pub use protocol::{
    AgentCapabilities, AgentDescriptor, Credentials, ErrorCode, MessageId, ProtocolCommand,
    ProtocolError, ProtocolRequest, ProtocolResponse, ProtocolResult, ProtocolValidationError,
    ResponseStatus,
};
pub use server::{AcceptedMessage, AgentRegistration, ConnectionInfo, IpcService, ServiceError};

pub const PROTOCOL_NAME: &str = "openpodium-ipc";
pub const PROTOCOL_VERSION: u16 = 1;
pub const SUPPORTED_VERSIONS: &[u16] = &[PROTOCOL_VERSION];
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const SECRET_FILE_NAME: &str = "ipc-secret";
