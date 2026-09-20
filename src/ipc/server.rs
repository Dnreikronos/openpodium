use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::portal::{PortalAction, PortalElementRef};

use super::store::{InsertResult, MessageStore, StoreError, StoredMessage};
use super::{
    AgentCapabilities, AgentDescriptor, AuthenticationError, CapabilityIssuer, Credentials,
    ErrorCode, MAX_FRAME_BYTES, MESSAGE_STORE_FILE_NAME, MessageId, PROTOCOL_NAME,
    PROTOCOL_VERSION, PortalActionRequest, PortalDispatcher, PortalServiceError, ProtocolCommand,
    ProtocolError, ProtocolRequest, ProtocolResponse, ProtocolResult, SECRET_FILE_NAME,
    SUPPORTED_VERSIONS,
};

const WORKER_COUNT: usize = 4;
const CONNECTION_QUEUE_CAPACITY: usize = 64;
const MESSAGE_CAPACITY: usize = 10_000;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRegistration {
    pub id: u64,
    pub name: String,
    pub program: String,
    pub state: String,
    pub capabilities: AgentCapabilities,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ConnectionInfo {
    endpoint: SocketAddr,
    credentials: Credentials,
}

impl ConnectionInfo {
    pub const fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }

    pub const fn workspace_id(&self) -> u64 {
        self.credentials.workspace_id
    }

    pub const fn agent_id(&self) -> u64 {
        self.credentials.agent_id
    }

    pub fn token(&self) -> &str {
        &self.credentials.token
    }

    pub fn credentials(&self) -> Credentials {
        self.credentials.clone()
    }
}

impl fmt::Debug for ConnectionInfo {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectionInfo")
            .field("endpoint", &self.endpoint)
            .field("workspace_id", &self.workspace_id())
            .field("agent_id", &self.agent_id())
            .field("token", &"[redacted]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedMessage {
    pub workspace_id: u64,
    pub sender_agent_id: u64,
    pub recipient_agent_id: u64,
    pub command: ProtocolCommand,
}

pub struct IpcService {
    endpoint: SocketAddr,
    issuer: Arc<CapabilityIssuer>,
    directory: Arc<RwLock<Directory>>,
    messages: Arc<Mutex<MessageStore>>,
    portals: Arc<Mutex<PortalDispatcher>>,
    leased_messages: Mutex<BTreeSet<(u64, MessageId)>>,
    shutdown: Arc<AtomicBool>,
    accept_thread: Option<JoinHandle<()>>,
    worker_threads: Vec<JoinHandle<()>>,
}

impl IpcService {
    pub fn start(data_directory: impl AsRef<Path>) -> Result<Self, ServiceError> {
        let data_directory = data_directory.as_ref();
        fs::create_dir_all(data_directory).map_err(|source| ServiceError::DataDirectory {
            path: data_directory.to_owned(),
            source,
        })?;
        let issuer = Arc::new(CapabilityIssuer::load_or_create(
            data_directory.join(SECRET_FILE_NAME),
        )?);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(ServiceError::Bind)?;
        listener
            .set_nonblocking(true)
            .map_err(ServiceError::Configure)?;
        let endpoint = listener.local_addr().map_err(ServiceError::Configure)?;

        let directory = Arc::new(RwLock::new(Directory::default()));
        let messages = Arc::new(Mutex::new(MessageStore::open(
            data_directory.join(MESSAGE_STORE_FILE_NAME),
        )?));
        let portals = Arc::new(Mutex::new(PortalDispatcher::open(data_directory)?));
        let shutdown = Arc::new(AtomicBool::new(false));
        let (connection_sender, connection_receiver) =
            mpsc::sync_channel(CONNECTION_QUEUE_CAPACITY);
        let connection_receiver = Arc::new(Mutex::new(connection_receiver));

        let mut worker_threads = Vec::with_capacity(WORKER_COUNT);
        for index in 0..WORKER_COUNT {
            let context = ServerContext {
                issuer: Arc::clone(&issuer),
                directory: Arc::clone(&directory),
                messages: Arc::clone(&messages),
                portals: Arc::clone(&portals),
            };
            let receiver = Arc::clone(&connection_receiver);
            let worker_shutdown = Arc::clone(&shutdown);
            worker_threads.push(
                thread::Builder::new()
                    .name(format!("openpodium-ipc-worker-{index}"))
                    .spawn(move || worker_loop(receiver, context, worker_shutdown))
                    .map_err(|source| ServiceError::Spawn {
                        thread: "worker",
                        source,
                    })?,
            );
        }
        let accept_shutdown = Arc::clone(&shutdown);
        let accept_thread = thread::Builder::new()
            .name("openpodium-ipc-accept".to_owned())
            .spawn(move || accept_loop(listener, connection_sender, accept_shutdown))
            .map_err(|source| ServiceError::Spawn {
                thread: "accept",
                source,
            })?;

        Ok(Self {
            endpoint,
            issuer,
            directory,
            messages,
            portals,
            leased_messages: Mutex::new(BTreeSet::new()),
            shutdown,
            accept_thread: Some(accept_thread),
            worker_threads,
        })
    }

    pub const fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }

    pub fn replace_workspace_agents(
        &self,
        workspace_id: u64,
        agents: impl IntoIterator<Item = AgentRegistration>,
    ) {
        let agents = agents.into_iter().map(|agent| (agent.id, agent)).collect();
        write_lock(&self.directory)
            .workspaces
            .insert(workspace_id, agents);
    }

    pub fn remove_workspace(&self, workspace_id: u64) {
        write_lock(&self.directory).workspaces.remove(&workspace_id);
    }

    pub fn register_portal<B>(
        &self,
        portal_id: u64,
        scope: super::PortalScope,
        config: crate::portal::PortalConfig,
        backend: B,
    ) -> Result<(), PortalServiceError>
    where
        B: crate::portal::PortalBackend + Send + 'static,
        B::Error: Send + Sync,
    {
        mutex_lock(&self.portals).register(portal_id, scope, config, backend)
    }

    pub fn attach_portal_agent(
        &self,
        portal_id: u64,
        agent_id: u64,
    ) -> Result<(), PortalServiceError> {
        mutex_lock(&self.portals).attach_agent(portal_id, agent_id)
    }

    pub fn connect_portal(&self, portal_id: u64) -> Result<(), PortalServiceError> {
        mutex_lock(&self.portals).connect(portal_id)
    }

    pub fn close_portal(&self, portal_id: u64) -> Result<(), PortalServiceError> {
        mutex_lock(&self.portals).close(portal_id)
    }

    pub fn connection_info(&self, workspace_id: u64, agent_id: u64) -> Option<ConnectionInfo> {
        let registered = read_lock(&self.directory)
            .workspaces
            .get(&workspace_id)
            .is_some_and(|agents| agents.contains_key(&agent_id));
        registered.then(|| ConnectionInfo {
            endpoint: self.endpoint,
            credentials: self.issuer.credentials(workspace_id, agent_id),
        })
    }

    pub fn try_recv(&self) -> Option<AcceptedMessage> {
        self.next_message().ok().flatten()
    }

    pub fn next_message(&self) -> Result<Option<AcceptedMessage>, ServiceError> {
        let messages = mutex_lock(&self.messages);
        let mut leased = mutex_lock(&self.leased_messages);
        let Some(message) = messages.next_pending(&leased)? else {
            return Ok(None);
        };
        leased.insert(message_key(&message));
        Ok(Some(message))
    }

    pub fn mark_processed(&self, message: &AcceptedMessage) -> Result<bool, ServiceError> {
        let message_id = message
            .command
            .message_id()
            .expect("accepted messages always have an ID");
        let processed =
            mutex_lock(&self.messages).mark_processed(message.workspace_id, message_id)?;
        mutex_lock(&self.leased_messages).remove(&message_key(message));
        Ok(processed)
    }

    pub fn release(&self, message: &AcceptedMessage) {
        mutex_lock(&self.leased_messages).remove(&message_key(message));
    }
}

impl Drop for IpcService {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
        for handle in self.worker_threads.drain(..) {
            let _ = handle.join();
        }
    }
}

#[derive(Default)]
struct Directory {
    workspaces: BTreeMap<u64, BTreeMap<u64, AgentRegistration>>,
}

struct ServerContext {
    issuer: Arc<CapabilityIssuer>,
    directory: Arc<RwLock<Directory>>,
    messages: Arc<Mutex<MessageStore>>,
    portals: Arc<Mutex<PortalDispatcher>>,
}

fn accept_loop(
    listener: TcpListener,
    connections: SyncSender<TcpStream>,
    shutdown: Arc<AtomicBool>,
) {
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                match connections.try_send(stream) {
                    Ok(()) => {}
                    Err(TrySendError::Full(mut stream)) => {
                        let response = ProtocolResponse::failure(
                            Some(PROTOCOL_VERSION),
                            None,
                            ProtocolError::new(
                                ErrorCode::ServiceUnavailable,
                                "the IPC service is busy; retry shortly",
                            ),
                        );
                        let _ = write_response(&mut stream, &response);
                    }
                    Err(TrySendError::Disconnected(_)) => return,
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(IDLE_POLL_INTERVAL);
            }
            Err(_) => return,
        }
    }
}

fn worker_loop(
    connections: Arc<Mutex<Receiver<TcpStream>>>,
    context: ServerContext,
    shutdown: Arc<AtomicBool>,
) {
    while !shutdown.load(Ordering::Acquire) {
        let received = mutex_lock(&connections).recv_timeout(IDLE_POLL_INTERVAL);
        match received {
            Ok(stream) => handle_connection(stream, &context),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn handle_connection(mut stream: TcpStream, context: &ServerContext) {
    let _ = stream.set_read_timeout(Some(CONNECTION_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CONNECTION_TIMEOUT));
    let response = match read_request(&mut stream) {
        Ok(request) => process_request(request, context),
        Err(error) => ProtocolResponse::failure(None, None, error),
    };
    let _ = write_response(&mut stream, &response);
}

fn read_request(stream: &mut TcpStream) -> Result<ProtocolRequest, ProtocolError> {
    let mut reader = BufReader::new(stream);
    let mut frame = Vec::new();
    let mut too_large = false;
    let terminated = loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(_) if too_large => break true,
            Err(error) => {
                return Err(ProtocolError::new(
                    ErrorCode::MalformedRequest,
                    format!("failed to read request: {error}"),
                ));
            }
        };
        if available.is_empty() {
            break false;
        }
        let consumed = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |position| position + 1);
        if !too_large {
            let retained = consumed.min(MAX_FRAME_BYTES + 1 - frame.len());
            frame.extend_from_slice(&available[..retained]);
            too_large = frame.len() > MAX_FRAME_BYTES;
        }
        let found_newline = available.get(consumed - 1) == Some(&b'\n');
        reader.consume(consumed);
        if found_newline {
            break true;
        }
    };
    if too_large {
        return Err(ProtocolError::new(
            ErrorCode::FrameTooLarge,
            format!("request exceeds the {MAX_FRAME_BYTES}-byte limit"),
        ));
    }
    if !terminated || frame.last() != Some(&b'\n') {
        return Err(ProtocolError::new(
            ErrorCode::MalformedRequest,
            "request must be terminated by a newline",
        ));
    }
    frame.pop();
    if frame.last() == Some(&b'\r') {
        frame.pop();
    }
    serde_json::from_slice(&frame).map_err(|error| {
        ProtocolError::new(
            ErrorCode::MalformedRequest,
            format!("request is not valid protocol JSON: {error}"),
        )
    })
}

fn process_request(request: ProtocolRequest, context: &ServerContext) -> ProtocolResponse {
    let request_id = request.request_id.clone();
    if request.protocol != PROTOCOL_NAME {
        return incompatible_response(
            Some(request_id),
            format!("unsupported protocol; expected {PROTOCOL_NAME:?}"),
        );
    }
    let Some(version) = request
        .supported_versions
        .iter()
        .copied()
        .find(|version| SUPPORTED_VERSIONS.contains(version))
    else {
        return incompatible_response(
            Some(request_id),
            "client protocol versions are incompatible; upgrade OpenPodium or its CLI".to_owned(),
        );
    };
    if !context.issuer.authenticates(&request.credentials)
        || !authenticated_agent_exists(&context.directory, &request.credentials)
    {
        return ProtocolResponse::failure(
            Some(version),
            Some(request_id),
            ProtocolError::new(ErrorCode::Unauthorized, "IPC authentication failed"),
        );
    }
    if request.command.minimum_version() > version {
        return ProtocolResponse::failure(
            Some(version),
            Some(request_id),
            ProtocolError::incompatible(
                format!(
                    "command requires protocol version {}; selected version is {version}",
                    request.command.minimum_version()
                ),
                SUPPORTED_VERSIONS.to_vec(),
            ),
        );
    }
    if let Err(error) = request.command.validate() {
        return ProtocolResponse::failure(
            Some(version),
            Some(request_id),
            ProtocolError::new(ErrorCode::InvalidRequest, error.to_string()),
        );
    }

    match execute_command(&request.credentials, request.command, context) {
        Ok(result) => ProtocolResponse::success(version, request_id, result),
        Err(error) => ProtocolResponse::failure(Some(version), Some(request_id), error),
    }
}

fn execute_command(
    credentials: &Credentials,
    command: ProtocolCommand,
    context: &ServerContext,
) -> Result<ProtocolResult, ProtocolError> {
    if command.is_portal() {
        return execute_portal_command(credentials, command, &context.portals);
    }
    if matches!(command, ProtocolCommand::ListAgents) {
        let directory = read_lock(&context.directory);
        let agents = directory
            .workspaces
            .get(&credentials.workspace_id)
            .into_iter()
            .flat_map(|agents| agents.values())
            .map(|agent| AgentDescriptor {
                id: agent.id,
                name: agent.name.clone(),
                program: agent.program.clone(),
                state: agent.state.clone(),
                is_self: agent.id == credentials.agent_id,
                capabilities: agent.capabilities.clone(),
            })
            .collect();
        return Ok(ProtocolResult::Agents { agents });
    }

    let message_id = command
        .message_id()
        .expect("non-list commands have message IDs")
        .clone();
    let mut messages = mutex_lock(&context.messages);
    if let Some(published) = messages
        .message(credentials.workspace_id, &message_id)
        .map_err(store_protocol_error)?
    {
        if published.sender_agent_id == credentials.agent_id && published.command == command {
            return Ok(ProtocolResult::Accepted {
                message_id,
                duplicate: true,
            });
        }
        return Err(ProtocolError::new(
            ErrorCode::IdempotencyConflict,
            "message ID was already used with a different sender or payload",
        ));
    }

    let recipient_agent_id = route_message(credentials, &command, &messages)?;
    if recipient_agent_id == credentials.agent_id {
        return Err(ProtocolError::new(
            ErrorCode::InvalidRequest,
            "sender and recipient agents must be different",
        ));
    }
    if !agent_is_visible(
        &context.directory,
        credentials.workspace_id,
        recipient_agent_id,
    ) {
        return Err(ProtocolError::new(
            ErrorCode::AgentNotVisible,
            "recipient agent is not visible in the authenticated workspace",
        ));
    }
    validate_capabilities(
        &context.directory,
        credentials.workspace_id,
        credentials.agent_id,
        recipient_agent_id,
        &command,
    )?;

    let accepted = AcceptedMessage {
        workspace_id: credentials.workspace_id,
        sender_agent_id: credentials.agent_id,
        recipient_agent_id,
        command: command.clone(),
    };
    match messages
        .insert(&accepted, MESSAGE_CAPACITY)
        .map_err(store_protocol_error)?
    {
        InsertResult::Inserted => {}
        InsertResult::Duplicate => {
            return Ok(ProtocolResult::Accepted {
                message_id,
                duplicate: true,
            });
        }
        InsertResult::Conflict => {
            return Err(ProtocolError::new(
                ErrorCode::IdempotencyConflict,
                "message ID was already used with a different sender or payload",
            ));
        }
        InsertResult::CapacityExhausted => {
            return Err(ProtocolError::new(
                ErrorCode::ServiceUnavailable,
                "the durable IPC message capacity is exhausted",
            ));
        }
    }
    Ok(ProtocolResult::Accepted {
        message_id,
        duplicate: false,
    })
}

fn execute_portal_command(
    credentials: &Credentials,
    command: ProtocolCommand,
    portals: &Mutex<PortalDispatcher>,
) -> Result<ProtocolResult, ProtocolError> {
    let mut portals = mutex_lock(portals);
    match command {
        ProtocolCommand::ListPortals => portals
            .list(credentials.workspace_id, credentials.agent_id)
            .map(|portals| ProtocolResult::Portals { portals })
            .map_err(portal_protocol_error),
        ProtocolCommand::InspectPortal { portal_id } => portals
            .inspect(credentials.workspace_id, credentials.agent_id, portal_id)
            .map(|inspection| ProtocolResult::PortalCapabilities {
                portal_id,
                descriptor: inspection.descriptor,
                capabilities: inspection.capabilities,
            })
            .map_err(portal_protocol_error),
        ProtocolCommand::ObservePortal { portal_id } => portals
            .observe(credentials.workspace_id, credentials.agent_id, portal_id)
            .map(|result| ProtocolResult::PortalObservation(result.observation))
            .map_err(portal_protocol_error),
        ProtocolCommand::RequestPortalAction {
            action_id,
            portal_id,
            action,
        } => {
            let action = portal_action(action)?;
            portals
                .request_action(
                    credentials.workspace_id,
                    credentials.agent_id,
                    action_id,
                    portal_id,
                    action,
                )
                .map(ProtocolResult::PortalReceipt)
                .map_err(portal_protocol_error)
        }
        ProtocolCommand::GetPortalResult { action_id } => portals
            .result(credentials.workspace_id, credentials.agent_id, &action_id)
            .map(ProtocolResult::PortalReceipt)
            .map_err(portal_protocol_error),
        _ => unreachable!("portal dispatcher received a non-portal command"),
    }
}

fn portal_action(action: PortalActionRequest) -> Result<PortalAction, ProtocolError> {
    match action {
        PortalActionRequest::Click {
            element_id,
            observation_revision,
        } => PortalElementRef::new(observation_revision, element_id)
            .map(PortalAction::Click)
            .map_err(|error| ProtocolError::new(ErrorCode::InvalidRequest, error.to_string())),
        PortalActionRequest::TypeText {
            element_id,
            observation_revision,
            text,
        } => PortalElementRef::new(observation_revision, element_id)
            .map(|element| PortalAction::TypeText { element, text })
            .map_err(|error| ProtocolError::new(ErrorCode::InvalidRequest, error.to_string())),
        PortalActionRequest::Scroll {
            element_id,
            observation_revision,
            delta_x,
            delta_y,
        } => element_id
            .map(|element_id| PortalElementRef::new(observation_revision, element_id))
            .transpose()
            .map(|element| PortalAction::Scroll {
                element,
                delta_x,
                delta_y,
            })
            .map_err(|error| ProtocolError::new(ErrorCode::InvalidRequest, error.to_string())),
        PortalActionRequest::Navigate { target } => Ok(PortalAction::Navigate(target)),
    }
}

fn portal_protocol_error(error: PortalServiceError) -> ProtocolError {
    let code = match error {
        PortalServiceError::WrongWorkspace { .. }
        | PortalServiceError::AgentNotAttached { .. }
        | PortalServiceError::NotConnected(_)
        | PortalServiceError::UnknownPortal(_) => ErrorCode::PortalUnavailable,
        PortalServiceError::ActionConflict(_) => ErrorCode::IdempotencyConflict,
        PortalServiceError::UnknownAction(_) => ErrorCode::UnknownPortalAction,
        PortalServiceError::Policy(crate::portal::PortalPolicyError::Denied(_)) => {
            ErrorCode::PortalPolicyDenied
        }
        PortalServiceError::Journal(_)
        | PortalServiceError::Backend(_)
        | PortalServiceError::Session(_)
        | PortalServiceError::MissingReceipt(_) => ErrorCode::ServiceUnavailable,
        PortalServiceError::InvalidPortalId
        | PortalServiceError::InvalidAgentId
        | PortalServiceError::InvalidScope
        | PortalServiceError::DuplicatePortal(_)
        | PortalServiceError::Policy(_) => ErrorCode::InvalidRequest,
    };
    ProtocolError::new(code, error.to_string())
}

fn validate_capabilities(
    directory: &RwLock<Directory>,
    workspace_id: u64,
    sender_agent_id: u64,
    recipient_agent_id: u64,
    command: &ProtocolCommand,
) -> Result<(), ProtocolError> {
    let directory = read_lock(directory);
    let agents = directory
        .workspaces
        .get(&workspace_id)
        .expect("authenticated workspaces remain registered during a request");
    let sender = agents
        .get(&sender_agent_id)
        .expect("authenticated agents remain registered during a request");
    let recipient = agents
        .get(&recipient_agent_id)
        .expect("visible recipients remain registered during a request");
    let (supported, message) = match command {
        ProtocolCommand::SendTask { .. }
        | ProtocolCommand::SendHandoff {
            kind: super::HandoffKind::Task,
            ..
        } => (
            recipient.capabilities.accepts_tasks,
            "recipient agent does not accept task handoffs",
        ),
        ProtocolCommand::SendHandoff {
            kind: super::HandoffKind::Question,
            ..
        } => (
            recipient.capabilities.accepts_questions,
            "recipient agent does not accept question handoffs",
        ),
        ProtocolCommand::ReportProgress { .. } | ProtocolCommand::ReportHandoffProgress { .. } => (
            sender.capabilities.reports_progress,
            "sending agent does not support progress updates",
        ),
        ProtocolCommand::Respond { .. } | ProtocolCommand::RespondToHandoff { .. } => (
            sender.capabilities.responds,
            "sending agent does not support responses",
        ),
        ProtocolCommand::CancelHandoff { .. } => (
            recipient.capabilities.supports_cancellation,
            "recipient agent does not support cancellation",
        ),
        ProtocolCommand::ListAgents
        | ProtocolCommand::ListPortals
        | ProtocolCommand::InspectPortal { .. }
        | ProtocolCommand::ObservePortal { .. }
        | ProtocolCommand::RequestPortalAction { .. }
        | ProtocolCommand::GetPortalResult { .. } => {
            unreachable!("read and portal commands are handled before routing")
        }
    };
    if supported {
        Ok(())
    } else {
        Err(ProtocolError::new(ErrorCode::InvalidRequest, message))
    }
}

fn route_message(
    credentials: &Credentials,
    command: &ProtocolCommand,
    messages: &MessageStore,
) -> Result<u64, ProtocolError> {
    match command {
        ProtocolCommand::SendTask {
            recipient_agent_id, ..
        } => Ok(*recipient_agent_id),
        ProtocolCommand::SendHandoff {
            recipient_agent_id,
            parent_message_id,
            ..
        } => {
            if let Some(parent_message_id) = parent_message_id {
                let parent =
                    handoff_message(messages, credentials.workspace_id, parent_message_id)?
                        .filter(|parent| {
                            parent.sender_agent_id == credentials.agent_id
                                || parent.recipient_agent_id == credentials.agent_id
                        })
                        .ok_or_else(|| {
                            ProtocolError::new(
                                ErrorCode::InvalidRequest,
                                "parent message does not identify a handoff involving this agent",
                            )
                        })?;
                let _ = parent;
            }
            Ok(*recipient_agent_id)
        }
        ProtocolCommand::ReportProgress {
            task_message_id, ..
        }
        | ProtocolCommand::Respond {
            task_message_id, ..
        } => {
            let task = messages
                .message(credentials.workspace_id, task_message_id)
                .map_err(store_protocol_error)?
                .filter(|message| {
                    message.recipient_agent_id == credentials.agent_id
                        && matches!(message.command, ProtocolCommand::SendTask { .. })
                })
                .ok_or_else(|| {
                    ProtocolError::new(
                        ErrorCode::InvalidRequest,
                        "task message does not identify a task assigned to this agent",
                    )
                })?;
            Ok(task.sender_agent_id)
        }
        ProtocolCommand::ReportHandoffProgress {
            handoff_message_id, ..
        }
        | ProtocolCommand::RespondToHandoff {
            handoff_message_id, ..
        } => {
            let handoff = handoff_message(messages, credentials.workspace_id, handoff_message_id)?
                .filter(|message| message.recipient_agent_id == credentials.agent_id)
                .ok_or_else(|| {
                    ProtocolError::new(
                        ErrorCode::InvalidRequest,
                        "handoff message does not identify work assigned to this agent",
                    )
                })?;
            Ok(handoff.sender_agent_id)
        }
        ProtocolCommand::CancelHandoff {
            handoff_message_id, ..
        } => {
            let handoff = handoff_message(messages, credentials.workspace_id, handoff_message_id)?
                .filter(|message| message.sender_agent_id == credentials.agent_id)
                .ok_or_else(|| {
                    ProtocolError::new(
                        ErrorCode::InvalidRequest,
                        "only the originating agent can cancel a handoff",
                    )
                })?;
            Ok(handoff.recipient_agent_id)
        }
        ProtocolCommand::ListAgents
        | ProtocolCommand::ListPortals
        | ProtocolCommand::InspectPortal { .. }
        | ProtocolCommand::ObservePortal { .. }
        | ProtocolCommand::RequestPortalAction { .. }
        | ProtocolCommand::GetPortalResult { .. } => {
            unreachable!("read and portal commands are handled before routing")
        }
    }
}

fn handoff_message(
    messages: &MessageStore,
    workspace_id: u64,
    message_id: &MessageId,
) -> Result<Option<StoredMessage>, ProtocolError> {
    Ok(messages
        .message(workspace_id, message_id)
        .map_err(store_protocol_error)?
        .filter(|message| {
            matches!(
                message.command,
                ProtocolCommand::SendTask { .. } | ProtocolCommand::SendHandoff { .. }
            )
        }))
}

fn store_protocol_error(error: StoreError) -> ProtocolError {
    ProtocolError::new(
        ErrorCode::ServiceUnavailable,
        format!("durable IPC state is unavailable: {error}"),
    )
}

fn message_key(message: &AcceptedMessage) -> (u64, MessageId) {
    (
        message.workspace_id,
        message
            .command
            .message_id()
            .expect("accepted messages always have an ID")
            .clone(),
    )
}

fn authenticated_agent_exists(directory: &RwLock<Directory>, credentials: &Credentials) -> bool {
    agent_is_visible(directory, credentials.workspace_id, credentials.agent_id)
}

fn agent_is_visible(directory: &RwLock<Directory>, workspace_id: u64, agent_id: u64) -> bool {
    read_lock(directory)
        .workspaces
        .get(&workspace_id)
        .is_some_and(|agents| agents.contains_key(&agent_id))
}

fn incompatible_response(request_id: Option<MessageId>, message: String) -> ProtocolResponse {
    ProtocolResponse::failure(
        None,
        request_id,
        ProtocolError::incompatible(message, SUPPORTED_VERSIONS.to_vec()),
    )
}

fn write_response(stream: &mut TcpStream, response: &ProtocolResponse) -> io::Result<()> {
    serde_json::to_writer(&mut *stream, response).map_err(io::Error::other)?;
    stream.write_all(b"\n")?;
    stream.flush()
}

fn read_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn write_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn mutex_lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Debug)]
pub enum ServiceError {
    DataDirectory {
        path: PathBuf,
        source: io::Error,
    },
    Authentication(AuthenticationError),
    Store(String),
    Portal(String),
    Bind(io::Error),
    Configure(io::Error),
    Spawn {
        thread: &'static str,
        source: io::Error,
    },
}

impl Display for ServiceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DataDirectory { path, source } => write!(
                formatter,
                "failed to create IPC data directory {}: {source}",
                path.display()
            ),
            Self::Authentication(source) => source.fmt(formatter),
            Self::Store(message) => formatter.write_str(message),
            Self::Portal(message) => formatter.write_str(message),
            Self::Bind(source) => write!(formatter, "failed to bind local IPC service: {source}"),
            Self::Configure(source) => {
                write!(formatter, "failed to configure local IPC service: {source}")
            }
            Self::Spawn { thread, source } => {
                write!(formatter, "failed to start IPC {thread} thread: {source}")
            }
        }
    }
}

impl Error for ServiceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::DataDirectory { source, .. }
            | Self::Bind(source)
            | Self::Configure(source)
            | Self::Spawn { source, .. } => Some(source),
            Self::Authentication(source) => Some(source),
            Self::Store(_) | Self::Portal(_) => None,
        }
    }
}

impl From<AuthenticationError> for ServiceError {
    fn from(value: AuthenticationError) -> Self {
        Self::Authentication(value)
    }
}

impl From<StoreError> for ServiceError {
    fn from(value: StoreError) -> Self {
        Self::Store(value.to_string())
    }
}

impl From<PortalServiceError> for ServiceError {
    fn from(value: PortalServiceError) -> Self {
        Self::Portal(value.to_string())
    }
}

#[cfg(test)]
mod tests;
