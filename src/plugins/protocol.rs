use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::catalog::{PluginRecord, PluginState};
use super::manifest::{Capability, PluginManifest, SdkVersion};

const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
const DEFAULT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostMessage {
    HostHello {
        plugin_id: String,
        sdk_version: SdkVersion,
        granted_capabilities: BTreeSet<Capability>,
    },
    InvokeCommand {
        id: u64,
        command: String,
        context: Value,
    },
    PrepareAdapter {
        id: u64,
        provider: String,
        request: AdapterRequest,
    },
    DeliverEvent {
        id: u64,
        name: String,
        payload: Value,
    },
    ProvideSettings {
        id: u64,
        values: BTreeMap<String, Value>,
    },
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginMessage {
    PluginReady { sdk_version: SdkVersion },
    CommandResult { id: u64, outcome: CommandOutcome },
    AdapterPlan { id: u64, plan: AdapterPlan },
    Acknowledged { id: u64 },
    Error { id: u64, message: String },
}

impl PluginMessage {
    fn response_id(&self) -> Option<u64> {
        match self {
            Self::PluginReady { .. } => None,
            Self::CommandResult { id, .. }
            | Self::AdapterPlan { id, .. }
            | Self::Acknowledged { id }
            | Self::Error { id, .. } => Some(*id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdapterRequest {
    pub working_directory: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterPlan {
    pub program: String,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandOutcome {
    #[serde(default)]
    pub value: Value,
    #[serde(default)]
    pub actions: Vec<PluginAction>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginAction {
    SetSetting { key: String, value: Value },
    ShowPanel { panel: String },
}

pub struct PluginSession {
    child: Child,
    input: BufWriter<ChildStdin>,
    responses: Receiver<Result<PluginMessage, PluginError>>,
    response_timeout: Duration,
    manifest: PluginManifest,
    sdk_version: SdkVersion,
    granted: BTreeSet<Capability>,
    next_request_id: u64,
    quarantined: bool,
}

impl PluginSession {
    pub fn start(record: &PluginRecord) -> Result<Self, PluginError> {
        Self::start_with_timeout(record, DEFAULT_RESPONSE_TIMEOUT)
    }

    pub fn start_with_timeout(
        record: &PluginRecord,
        response_timeout: Duration,
    ) -> Result<Self, PluginError> {
        if record.state() != PluginState::Enabled {
            return Err(PluginError::NotEnabled(record.manifest().id.clone()));
        }
        let sdk_version = record
            .negotiated_sdk()
            .ok_or_else(|| PluginError::Incompatible(record.manifest().id.clone()))?;
        let mut command = Command::new(record.executable());
        command
            .current_dir(record.directory())
            .env_clear()
            .env("OPENPODIUM_PLUGIN_ID", &record.manifest().id)
            .env("OPENPODIUM_PLUGIN_SDK", sdk_version.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for name in [
            "PATH",
            "PATHEXT",
            "SYSTEMROOT",
            "WINDIR",
            "TEMP",
            "TMP",
            "TMPDIR",
            "LANG",
            "LC_ALL",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        let mut child = command
            .spawn()
            .map_err(|error| PluginError::Start(error.to_string()))?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| PluginError::Start("plugin standard input is unavailable".to_owned()))?;
        let output = child.stdout.take().ok_or_else(|| {
            PluginError::Start("plugin standard output is unavailable".to_owned())
        })?;
        let (response_sender, responses) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name(format!("plugin-reader-{}", record.manifest().id))
            .spawn(move || {
                let mut output = BufReader::new(output);
                loop {
                    let response = read_message(&mut output);
                    let terminal = response.is_err();
                    if response_sender.send(response).is_err() || terminal {
                        break;
                    }
                }
            })
            .map_err(|error| {
                let _ = child.kill();
                let _ = child.wait();
                PluginError::Start(format!("could not start plugin reader: {error}"))
            })?;
        let granted = record.granted().capabilities().collect();
        let mut session = Self {
            child,
            input: BufWriter::new(input),
            responses,
            response_timeout,
            manifest: record.manifest().clone(),
            sdk_version,
            granted,
            next_request_id: 1,
            quarantined: false,
        };
        let hello = HostMessage::HostHello {
            plugin_id: session.manifest.id.clone(),
            sdk_version,
            granted_capabilities: session.granted.clone(),
        };
        if let Err(error) = write_message(&mut session.input, &hello) {
            session.quarantine();
            return Err(error);
        }
        let response = match session.receive() {
            Ok(response) => response,
            Err(error) => {
                session.quarantine();
                return Err(error);
            }
        };
        match response {
            PluginMessage::PluginReady {
                sdk_version: plugin_version,
            } if plugin_version == sdk_version => Ok(session),
            PluginMessage::PluginReady {
                sdk_version: plugin_version,
            } => {
                session.quarantine();
                Err(PluginError::Protocol(format!(
                    "plugin accepted SDK {plugin_version}, expected {sdk_version}"
                )))
            }
            _ => {
                session.quarantine();
                Err(PluginError::Protocol(
                    "plugin did not answer the handshake with plugin_ready".to_owned(),
                ))
            }
        }
    }

    pub const fn sdk_version(&self) -> SdkVersion {
        self.sdk_version
    }

    pub const fn quarantined(&self) -> bool {
        self.quarantined
    }

    pub fn invoke_command(
        &mut self,
        command: &str,
        context: Value,
    ) -> Result<CommandOutcome, PluginError> {
        self.require(Capability::Commands)?;
        if !self
            .manifest
            .contributions
            .commands
            .iter()
            .any(|candidate| candidate.id == command)
        {
            return Err(PluginError::UnknownContribution(command.to_owned()));
        }
        let id = self.take_request_id()?;
        let response = self.exchange(
            id,
            HostMessage::InvokeCommand {
                id,
                command: command.to_owned(),
                context,
            },
        )?;
        match response {
            PluginMessage::CommandResult { outcome, .. } => {
                self.validate_actions(&outcome.actions)?;
                Ok(outcome)
            }
            PluginMessage::Error { message, .. } => Err(PluginError::Plugin(message)),
            _ => self.protocol_failure("plugin returned the wrong command response"),
        }
    }

    pub fn prepare_adapter(
        &mut self,
        provider: &str,
        request: AdapterRequest,
    ) -> Result<AdapterPlan, PluginError> {
        self.require(Capability::Adapters)?;
        if !self
            .manifest
            .contributions
            .providers
            .iter()
            .any(|candidate| candidate.id == provider)
        {
            return Err(PluginError::UnknownContribution(provider.to_owned()));
        }
        let id = self.take_request_id()?;
        let response = self.exchange(
            id,
            HostMessage::PrepareAdapter {
                id,
                provider: provider.to_owned(),
                request,
            },
        )?;
        match response {
            PluginMessage::AdapterPlan { plan, .. } if !plan.program.trim().is_empty() => Ok(plan),
            PluginMessage::AdapterPlan { .. } => {
                self.protocol_failure("plugin returned an empty adapter program")
            }
            PluginMessage::Error { message, .. } => Err(PluginError::Plugin(message)),
            _ => self.protocol_failure("plugin returned the wrong adapter response"),
        }
    }

    pub fn deliver_event(&mut self, name: &str, payload: Value) -> Result<(), PluginError> {
        self.require(Capability::Events)?;
        if !self
            .manifest
            .contributions
            .events
            .iter()
            .any(|event| event == name)
        {
            return Err(PluginError::UnknownContribution(name.to_owned()));
        }
        let id = self.take_request_id()?;
        match self.exchange(
            id,
            HostMessage::DeliverEvent {
                id,
                name: name.to_owned(),
                payload,
            },
        )? {
            PluginMessage::Acknowledged { .. } => Ok(()),
            PluginMessage::Error { message, .. } => Err(PluginError::Plugin(message)),
            _ => self.protocol_failure("plugin did not acknowledge the event"),
        }
    }

    pub fn provide_settings(&mut self, values: BTreeMap<String, Value>) -> Result<(), PluginError> {
        self.require(Capability::SettingsRead)?;
        if let Some(key) = values.keys().find(|key| {
            !self
                .manifest
                .contributions
                .settings
                .iter()
                .any(|setting| &setting.key == *key)
        }) {
            return Err(PluginError::UnknownContribution(key.clone()));
        }
        let id = self.take_request_id()?;
        match self.exchange(id, HostMessage::ProvideSettings { id, values })? {
            PluginMessage::Acknowledged { .. } => Ok(()),
            PluginMessage::Error { message, .. } => Err(PluginError::Plugin(message)),
            _ => self.protocol_failure("plugin did not acknowledge settings"),
        }
    }

    pub fn shutdown(mut self) {
        let _ = write_message(&mut self.input, &HostMessage::Shutdown);
        self.stop_child();
    }

    fn require(&self, capability: Capability) -> Result<(), PluginError> {
        if self.granted.contains(&capability) {
            Ok(())
        } else {
            Err(PluginError::PermissionDenied(capability))
        }
    }

    fn validate_actions(&mut self, actions: &[PluginAction]) -> Result<(), PluginError> {
        for action in actions {
            match action {
                PluginAction::SetSetting { key, .. } => {
                    if !self.granted.contains(&Capability::SettingsWrite) {
                        return self.protocol_failure(
                            "plugin proposed a setting change without settings.write",
                        );
                    }
                    if !self
                        .manifest
                        .contributions
                        .settings
                        .iter()
                        .any(|setting| setting.key == *key)
                    {
                        return self.protocol_failure(
                            "plugin proposed a change for an undeclared setting",
                        );
                    }
                }
                PluginAction::ShowPanel { panel } => {
                    if !self.granted.contains(&Capability::Ui) {
                        return self.protocol_failure("plugin requested UI without ui permission");
                    }
                    if !self
                        .manifest
                        .contributions
                        .panels
                        .iter()
                        .any(|candidate| candidate.id == *panel)
                    {
                        return self.protocol_failure("plugin requested an undeclared panel");
                    }
                }
            }
        }
        Ok(())
    }

    fn take_request_id(&mut self) -> Result<u64, PluginError> {
        if self.quarantined {
            return Err(PluginError::Quarantined);
        }
        let id = self.next_request_id;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .ok_or_else(|| PluginError::Protocol("request identifier overflow".to_owned()))?;
        Ok(id)
    }

    fn exchange(&mut self, id: u64, message: HostMessage) -> Result<PluginMessage, PluginError> {
        if let Err(error) = write_message(&mut self.input, &message) {
            if error != PluginError::RequestTooLarge {
                self.quarantine();
            }
            return Err(error);
        }
        let response = match self.receive() {
            Ok(response) => response,
            Err(error) => {
                self.quarantine();
                return Err(error);
            }
        };
        if response.response_id() != Some(id) {
            return self.protocol_failure("plugin response identifier did not match the request");
        }
        Ok(response)
    }

    fn receive(&self) -> Result<PluginMessage, PluginError> {
        match self.responses.recv_timeout(self.response_timeout) {
            Ok(response) => response,
            Err(RecvTimeoutError::Timeout) => Err(PluginError::TimedOut),
            Err(RecvTimeoutError::Disconnected) => Err(PluginError::Exited),
        }
    }

    fn protocol_failure<T>(&mut self, message: &str) -> Result<T, PluginError> {
        self.quarantine();
        Err(PluginError::Protocol(message.to_owned()))
    }

    fn quarantine(&mut self) {
        self.quarantined = true;
        self.stop_child();
    }

    fn stop_child(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for PluginSession {
    fn drop(&mut self) {
        self.stop_child();
    }
}

fn write_message(
    input: &mut BufWriter<ChildStdin>,
    message: &HostMessage,
) -> Result<(), PluginError> {
    let encoded = serde_json::to_vec(message).map_err(|error| {
        PluginError::Protocol(format!("could not encode host message: {error}"))
    })?;
    if encoded.len() + 1 > MAX_MESSAGE_BYTES {
        return Err(PluginError::RequestTooLarge);
    }
    input
        .write_all(&encoded)
        .and_then(|()| input.write_all(b"\n"))
        .and_then(|()| input.flush())
        .map_err(io_error)
}

fn read_message(output: &mut impl BufRead) -> Result<PluginMessage, PluginError> {
    let mut line = String::new();
    let bytes = output
        .take((MAX_MESSAGE_BYTES + 1) as u64)
        .read_line(&mut line)
        .map_err(io_error)?;
    if bytes == 0 {
        return Err(PluginError::Exited);
    }
    if bytes > MAX_MESSAGE_BYTES || !line.ends_with('\n') {
        return Err(PluginError::Protocol(
            "plugin message exceeded the size limit or was not newline terminated".to_owned(),
        ));
    }
    serde_json::from_str(&line)
        .map_err(|error| PluginError::Protocol(format!("plugin returned invalid JSON: {error}")))
}

fn io_error(error: io::Error) -> PluginError {
    PluginError::Io(error.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginError {
    NotEnabled(String),
    Incompatible(String),
    Start(String),
    PermissionDenied(Capability),
    UnknownContribution(String),
    Plugin(String),
    Protocol(String),
    Io(String),
    RequestTooLarge,
    TimedOut,
    Exited,
    Quarantined,
}

impl Display for PluginError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotEnabled(plugin_id) => write!(formatter, "plugin `{plugin_id}` is not enabled"),
            Self::Incompatible(plugin_id) => {
                write!(
                    formatter,
                    "plugin `{plugin_id}` is incompatible with this SDK"
                )
            }
            Self::Start(message) => write!(formatter, "could not start plugin: {message}"),
            Self::PermissionDenied(capability) => {
                write!(formatter, "plugin was not granted `{capability}`")
            }
            Self::UnknownContribution(identifier) => {
                write!(
                    formatter,
                    "plugin contribution `{identifier}` is not declared"
                )
            }
            Self::Plugin(message) => write!(formatter, "plugin returned an error: {message}"),
            Self::Protocol(message) => write!(formatter, "plugin protocol failure: {message}"),
            Self::Io(message) => write!(formatter, "plugin I/O failure: {message}"),
            Self::RequestTooLarge => formatter.write_str("plugin request exceeded the size limit"),
            Self::TimedOut => formatter.write_str("plugin did not respond before the timeout"),
            Self::Exited => formatter.write_str("plugin exited before responding"),
            Self::Quarantined => formatter.write_str("plugin session is quarantined"),
        }
    }
}

impl Error for PluginError {}
