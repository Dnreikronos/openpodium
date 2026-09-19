use std::env;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::time::Duration;

use super::{
    Credentials, MAX_FRAME_BYTES, MessageId, PROTOCOL_NAME, ProtocolCommand, ProtocolRequest,
    ProtocolResponse, SUPPORTED_VERSIONS,
};

pub const AVAILABLE_ENV: &str = "OPENPODIUM_IPC_AVAILABLE";
pub const ENDPOINT_ENV: &str = "OPENPODIUM_IPC_ENDPOINT";
pub const TOKEN_ENV: &str = "OPENPODIUM_IPC_TOKEN";
pub const VERSIONS_ENV: &str = "OPENPODIUM_IPC_VERSIONS";
pub const WORKSPACE_ID_ENV: &str = "OPENPODIUM_WORKSPACE_ID";
pub const AGENT_ID_ENV: &str = "OPENPODIUM_AGENT_ID";
pub const CLI_ENV: &str = "OPENPODIUM_CLI";

const IO_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, PartialEq, Eq)]
pub struct ConnectionConfig {
    endpoint: SocketAddr,
    credentials: Credentials,
}

impl ConnectionConfig {
    pub fn from_environment() -> Result<Self, ClientError> {
        Self::from_lookup(|name| env::var(name).ok())
    }

    pub const fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }

    pub fn credentials(&self) -> Credentials {
        self.credentials.clone()
    }

    fn from_lookup(mut lookup: impl FnMut(&str) -> Option<String>) -> Result<Self, ClientError> {
        if lookup(AVAILABLE_ENV).as_deref() == Some("0") {
            return Err(ClientError::Unavailable(
                "IPC is unavailable in this agent environment; local transport forwarding is not configured"
                    .to_owned(),
            ));
        }
        let endpoint: SocketAddr =
            required_value(&mut lookup, ENDPOINT_ENV)?
                .parse()
                .map_err(|_| ClientError::InvalidEnvironment {
                    variable: ENDPOINT_ENV,
                    message: "must be an IPv4 socket address such as 127.0.0.1:43123".to_owned(),
                })?;
        if !endpoint.ip().is_loopback() {
            return Err(ClientError::InvalidEnvironment {
                variable: ENDPOINT_ENV,
                message: "must use a loopback address so credentials never leave this machine"
                    .to_owned(),
            });
        }
        let workspace_id = parse_positive_id(&mut lookup, WORKSPACE_ID_ENV)?;
        let agent_id = parse_positive_id(&mut lookup, AGENT_ID_ENV)?;
        let token = required_value(&mut lookup, TOKEN_ENV)?;
        Ok(Self {
            endpoint,
            credentials: Credentials {
                workspace_id,
                agent_id,
                token,
            },
        })
    }
}

impl fmt::Debug for ConnectionConfig {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectionConfig")
            .field("endpoint", &self.endpoint)
            .field("credentials", &self.credentials)
            .finish()
    }
}

pub struct IpcClient {
    config: ConnectionConfig,
}

impl IpcClient {
    pub const fn new(config: ConnectionConfig) -> Self {
        Self { config }
    }

    pub fn send(
        &self,
        request_id: MessageId,
        command: ProtocolCommand,
    ) -> Result<ProtocolResponse, ClientError> {
        let request = ProtocolRequest::new(request_id, self.config.credentials(), command);
        let mut stream = TcpStream::connect_timeout(&self.config.endpoint, IO_TIMEOUT)
            .map_err(ClientError::Connect)?;
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .map_err(ClientError::Configure)?;
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .map_err(ClientError::Configure)?;
        serde_json::to_writer(&mut stream, &request).map_err(ClientError::Encode)?;
        stream.write_all(b"\n").map_err(ClientError::Write)?;
        stream.flush().map_err(ClientError::Write)?;
        stream
            .shutdown(Shutdown::Write)
            .map_err(ClientError::Write)?;

        let mut frame = Vec::new();
        let limit = u64::try_from(MAX_FRAME_BYTES + 1).expect("frame limit fits in u64");
        BufReader::new(stream.take(limit))
            .read_until(b'\n', &mut frame)
            .map_err(ClientError::Read)?;
        if frame.len() > MAX_FRAME_BYTES {
            return Err(ClientError::InvalidResponse(format!(
                "response exceeds the {MAX_FRAME_BYTES}-byte limit"
            )));
        }
        if frame.last() != Some(&b'\n') {
            return Err(ClientError::InvalidResponse(
                "response is not terminated by a newline".to_owned(),
            ));
        }
        let response: ProtocolResponse =
            serde_json::from_slice(&frame).map_err(ClientError::Decode)?;
        if response.protocol != PROTOCOL_NAME {
            return Err(ClientError::InvalidResponse(format!(
                "server returned protocol {:?}; expected {PROTOCOL_NAME:?}",
                response.protocol
            )));
        }
        if response.error.is_none()
            && !response
                .version
                .is_some_and(|version| SUPPORTED_VERSIONS.contains(&version))
        {
            return Err(ClientError::InvalidResponse(format!(
                "server returned unsupported version {:?}; expected one of {SUPPORTED_VERSIONS:?}",
                response.version,
            )));
        }
        Ok(response)
    }
}

fn required_value(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    variable: &'static str,
) -> Result<String, ClientError> {
    lookup(variable)
        .filter(|value| !value.is_empty())
        .ok_or(ClientError::MissingEnvironment { variable })
}

fn parse_positive_id(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    variable: &'static str,
) -> Result<u64, ClientError> {
    required_value(lookup, variable)?
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| ClientError::InvalidEnvironment {
            variable,
            message: "must be a positive integer".to_owned(),
        })
}

#[derive(Debug)]
pub enum ClientError {
    MissingEnvironment {
        variable: &'static str,
    },
    InvalidEnvironment {
        variable: &'static str,
        message: String,
    },
    Unavailable(String),
    Connect(io::Error),
    Configure(io::Error),
    Encode(serde_json::Error),
    Write(io::Error),
    Read(io::Error),
    Decode(serde_json::Error),
    InvalidResponse(String),
}

impl ClientError {
    pub const fn is_environment_error(&self) -> bool {
        matches!(
            self,
            Self::MissingEnvironment { .. }
                | Self::InvalidEnvironment { .. }
                | Self::Unavailable(_)
        )
    }
}

impl Display for ClientError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingEnvironment { variable } => write!(
                formatter,
                "{variable} is not set; run this command from an OpenPodium-launched local agent"
            ),
            Self::InvalidEnvironment { variable, message } => {
                write!(formatter, "{variable} {message}")
            }
            Self::Unavailable(message) | Self::InvalidResponse(message) => {
                formatter.write_str(message)
            }
            Self::Connect(source) => write!(formatter, "failed to connect to OpenPodium: {source}"),
            Self::Configure(source) => {
                write!(
                    formatter,
                    "failed to configure the IPC connection: {source}"
                )
            }
            Self::Encode(source) => write!(formatter, "failed to encode IPC request: {source}"),
            Self::Write(source) => write!(formatter, "failed to write IPC request: {source}"),
            Self::Read(source) => write!(formatter, "failed to read IPC response: {source}"),
            Self::Decode(source) => write!(formatter, "failed to decode IPC response: {source}"),
        }
    }
}

impl Error for ClientError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Connect(source)
            | Self::Configure(source)
            | Self::Write(source)
            | Self::Read(source) => Some(source),
            Self::Encode(source) | Self::Decode(source) => Some(source),
            Self::MissingEnvironment { .. }
            | Self::InvalidEnvironment { .. }
            | Self::Unavailable(_)
            | Self::InvalidResponse(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tempfile::TempDir;

    use super::*;
    use crate::ipc::{AgentCapabilities, AgentRegistration, IpcService, ProtocolResult};

    #[test]
    fn environment_discovery_is_strict_and_never_accepts_cli_tokens() {
        let values = BTreeMap::from([
            (ENDPOINT_ENV, "127.0.0.1:43210".to_owned()),
            (WORKSPACE_ID_ENV, "4".to_owned()),
            (AGENT_ID_ENV, "7".to_owned()),
            (TOKEN_ENV, "secret".to_owned()),
        ]);
        let config = ConnectionConfig::from_lookup(|name| values.get(name).cloned()).unwrap();
        assert_eq!(config.endpoint().to_string(), "127.0.0.1:43210");
        assert_eq!(config.credentials().workspace_id, 4);
        assert_eq!(config.credentials().agent_id, 7);
        assert!(!format!("{config:?}").contains("secret"));

        let unavailable =
            ConnectionConfig::from_lookup(|name| (name == AVAILABLE_ENV).then(|| "0".to_owned()));
        assert!(matches!(unavailable, Err(ClientError::Unavailable(_))));

        let mut non_local = values;
        non_local.insert(ENDPOINT_ENV, "192.0.2.1:43210".to_owned());
        assert!(matches!(
            ConnectionConfig::from_lookup(|name| non_local.get(name).cloned()),
            Err(ClientError::InvalidEnvironment {
                variable: ENDPOINT_ENV,
                ..
            })
        ));
    }

    #[test]
    fn client_round_trips_against_the_local_service() {
        let temp = TempDir::new().unwrap();
        let service = IpcService::start(temp.path()).unwrap();
        service.replace_workspace_agents(
            4,
            [AgentRegistration {
                id: 7,
                name: "Builder".to_owned(),
                program: "Codex".to_owned(),
                state: "running".to_owned(),
                capabilities: AgentCapabilities::CONNECTED,
            }],
        );
        let info = service.connection_info(4, 7).unwrap();
        let client = IpcClient::new(ConnectionConfig {
            endpoint: info.endpoint(),
            credentials: info.credentials(),
        });

        let response = client
            .send(
                MessageId::new("request-1").unwrap(),
                ProtocolCommand::ListAgents,
            )
            .unwrap();

        assert!(matches!(
            response.result,
            Some(ProtocolResult::Agents { agents }) if agents.len() == 1
        ));
    }
}
