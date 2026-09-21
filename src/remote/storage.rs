use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::crypto::{DeviceIdentity, SecretKey};
use super::protocol::{
    CommandId, CommandOutcome, CommandReceipt, ConnectionDiagnostic, DeviceId, PeerGrant,
    SessionId, SteeringCommand, StreamDelta,
};
use super::session::{
    CommandDisposition, CommandLedger, CommandRecord, DeviceRegistry, EncryptedPacket,
    InboundPacket, MAX_OUTBOX_PACKETS, PairedDevice, RemoteError, RemoteSession, SessionCheckpoint,
};

pub const REMOTE_STATE_FILE_NAME: &str = "remote-state.sqlite";
const STATE_FORMAT_VERSION: u16 = 1;
const MAX_STATE_BYTES: usize = 64 * 1024 * 1024;
const MAX_PAIRED_DEVICES: usize = 256;
const MAX_REVOKED_DEVICES: usize = 4096;
const MAX_SESSIONS: usize = 1024;
const MAX_COMMAND_RECORDS: usize = 100_000;
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS remote_state (
    singleton      INTEGER PRIMARY KEY CHECK (singleton = 1),
    format_version INTEGER NOT NULL,
    payload        BLOB NOT NULL
) STRICT;
";

pub struct RemoteState {
    identity: DeviceIdentity,
    registry: DeviceRegistry,
    sessions: BTreeMap<SessionId, SessionCheckpoint>,
    ledger: CommandLedger,
}

impl RemoteState {
    pub fn new(identity: DeviceIdentity) -> Self {
        Self {
            identity,
            registry: DeviceRegistry::default(),
            sessions: BTreeMap::new(),
            ledger: CommandLedger::default(),
        }
    }

    pub const fn identity(&self) -> &DeviceIdentity {
        &self.identity
    }

    pub const fn registry(&self) -> &DeviceRegistry {
        &self.registry
    }

    pub const fn ledger(&self) -> &CommandLedger {
        &self.ledger
    }

    fn begin_command(
        &mut self,
        device_id: DeviceId,
        command: &SteeringCommand,
    ) -> Result<CommandDisposition, RemoteError> {
        self.ledger.begin(&self.registry, device_id, command)
    }

    fn complete_command(
        &mut self,
        command_id: CommandId,
        outcome: CommandOutcome,
    ) -> Result<CommandReceipt, RemoteError> {
        self.ledger.complete(command_id, outcome)
    }

    pub fn checkpoint(&self, session_id: SessionId) -> Option<&SessionCheckpoint> {
        self.sessions.get(&session_id)
    }

    fn put_checkpoint(&mut self, checkpoint: SessionCheckpoint) -> Result<(), RemoteError> {
        if checkpoint.local_id != self.identity.id()
            || self.registry.device(checkpoint.peer_id).is_none()
        {
            return Err(RemoteError::InvalidPacket(
                "checkpoint identities are not paired",
            ));
        }
        if let Some(previous) = self.sessions.get(&checkpoint.session_id)
            && (checkpoint.local_id != previous.local_id
                || checkpoint.peer_id != previous.peer_id
                || checkpoint.root.bytes() != previous.root.bytes()
                || checkpoint.next_nonce < previous.next_nonce
                || checkpoint.next_delivery < previous.next_delivery
                || checkpoint.committed_inbound < previous.committed_inbound)
        {
            return Err(RemoteError::InvalidPacket(
                "checkpoint would roll back a session",
            ));
        }
        if !self.sessions.contains_key(&checkpoint.session_id)
            && self.sessions.len() >= MAX_SESSIONS
        {
            return Err(RemoteError::InvalidPayload("session limit reached"));
        }
        self.sessions.insert(checkpoint.session_id, checkpoint);
        Ok(())
    }

    fn encode(&self) -> Result<Vec<u8>, RemoteStorageError> {
        let wire = RemoteStateWire {
            identity: IdentityWire {
                id: self.identity.id,
                name: self.identity.name.clone(),
                secret: self.identity.secret_bytes(),
            },
            devices: self
                .registry
                .devices
                .values()
                .map(|device| PairedDeviceWire {
                    local_id: device.local_id,
                    id: device.id,
                    name: device.name.clone(),
                    public_key: device.public_key,
                    root: device.root.copy_bytes(),
                    grant: device.grant.clone(),
                })
                .collect(),
            revoked: self.registry.revoked.clone(),
            sessions: self
                .sessions
                .values()
                .map(|checkpoint| SessionWire {
                    session_id: checkpoint.session_id,
                    local_id: checkpoint.local_id,
                    peer_id: checkpoint.peer_id,
                    root: checkpoint.root.copy_bytes(),
                    next_nonce: checkpoint.next_nonce,
                    next_delivery: checkpoint.next_delivery,
                    committed_inbound: checkpoint.committed_inbound,
                    outbox: checkpoint.outbox.clone(),
                })
                .collect(),
            ledger: self
                .ledger
                .records
                .iter()
                .map(|(id, record)| (*id, record.clone()))
                .collect(),
        };
        let bytes = serde_json::to_vec(&wire).map_err(RemoteStorageError::Encode)?;
        if bytes.len() > MAX_STATE_BYTES {
            return Err(RemoteStorageError::StateTooLarge);
        }
        Ok(bytes)
    }

    fn decode(bytes: &[u8]) -> Result<Self, RemoteStorageError> {
        if bytes.len() > MAX_STATE_BYTES {
            return Err(RemoteStorageError::StateTooLarge);
        }
        let wire: RemoteStateWire =
            serde_json::from_slice(bytes).map_err(RemoteStorageError::Decode)?;
        if wire.devices.len() > MAX_PAIRED_DEVICES
            || wire.revoked.len() > MAX_REVOKED_DEVICES
            || wire.sessions.len() > MAX_SESSIONS
            || wire.ledger.len() > MAX_COMMAND_RECORDS
        {
            return Err(RemoteStorageError::InvalidState(
                "remote state collection exceeds its limit",
            ));
        }
        let identity =
            DeviceIdentity::restore(wire.identity.id, wire.identity.name, wire.identity.secret)?;
        let mut devices = BTreeMap::new();
        for device in wire.devices {
            if device.name.trim().is_empty() || device.name.chars().count() > 128 {
                return Err(RemoteStorageError::InvalidState(
                    "paired device name is invalid",
                ));
            }
            if wire.revoked.contains(&device.id) {
                return Err(RemoteStorageError::InvalidState(
                    "device cannot be paired and revoked",
                ));
            }
            if device.local_id != identity.id() {
                return Err(RemoteStorageError::InvalidState(
                    "paired device belongs to another local identity",
                ));
            }
            let id = device.id;
            let paired = PairedDevice::new(
                device.local_id,
                id,
                device.name,
                device.public_key,
                SecretKey::new(device.root),
                device.grant,
            );
            if devices.insert(id, paired).is_some() {
                return Err(RemoteStorageError::InvalidState(
                    "paired device identifier is duplicated",
                ));
            }
        }
        let mut sessions = BTreeMap::new();
        for session in wire.sessions {
            validate_session(&session, identity.id(), &devices)?;
            let session_id = session.session_id;
            let checkpoint = SessionCheckpoint {
                session_id,
                local_id: session.local_id,
                peer_id: session.peer_id,
                root: SecretKey::new(session.root),
                next_nonce: session.next_nonce,
                next_delivery: session.next_delivery,
                committed_inbound: session.committed_inbound,
                outbox: session.outbox,
            };
            if sessions.insert(session_id, checkpoint).is_some() {
                return Err(RemoteStorageError::InvalidState(
                    "session identifier is duplicated",
                ));
            }
        }
        let ledger: BTreeMap<_, _> = wire.ledger.iter().cloned().collect();
        if ledger.len() != wire.ledger.len() {
            return Err(RemoteStorageError::InvalidState(
                "command identifier is duplicated",
            ));
        }
        Ok(Self {
            identity,
            registry: DeviceRegistry {
                devices,
                revoked: wire.revoked,
            },
            sessions,
            ledger: CommandLedger { records: ledger },
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoteStateWire {
    identity: IdentityWire,
    devices: Vec<PairedDeviceWire>,
    revoked: BTreeSet<DeviceId>,
    sessions: Vec<SessionWire>,
    ledger: Vec<(CommandId, CommandRecord)>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityWire {
    id: DeviceId,
    name: String,
    secret: [u8; 32],
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairedDeviceWire {
    local_id: DeviceId,
    id: DeviceId,
    name: String,
    public_key: [u8; 32],
    root: [u8; 32],
    grant: PeerGrant,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionWire {
    session_id: SessionId,
    local_id: DeviceId,
    peer_id: DeviceId,
    root: [u8; 32],
    next_nonce: u64,
    next_delivery: u64,
    committed_inbound: u64,
    outbox: BTreeMap<u64, EncryptedPacket>,
}

fn validate_session(
    session: &SessionWire,
    local_id: DeviceId,
    devices: &BTreeMap<DeviceId, PairedDevice>,
) -> Result<(), RemoteStorageError> {
    if session.local_id != local_id || session.next_nonce == 0 || session.next_delivery == 0 {
        return Err(RemoteStorageError::InvalidState(
            "session counters or local identity are invalid",
        ));
    }
    let peer = devices
        .get(&session.peer_id)
        .ok_or(RemoteStorageError::InvalidState(
            "session peer is not paired",
        ))?;
    if peer.root.bytes() != &session.root || session.outbox.len() > MAX_OUTBOX_PACKETS {
        return Err(RemoteStorageError::InvalidState(
            "session key or outbox is invalid",
        ));
    }
    for (sequence, packet) in &session.outbox {
        if *sequence == 0
            || *sequence >= session.next_delivery
            || packet.header.version != super::PROTOCOL_VERSION
            || packet.header.session_id != session.session_id
            || packet.header.sender_id != local_id
            || packet.header.delivery_sequence != Some(*sequence)
            || packet.header.nonce_counter == 0
            || packet.header.nonce_counter >= session.next_nonce
            || packet.ciphertext.len() > super::MAX_PACKET_BYTES + 16
        {
            return Err(RemoteStorageError::InvalidState(
                "session outbox contains an invalid packet",
            ));
        }
    }
    Ok(())
}

pub struct RemoteStateStore {
    connection: Connection,
    path: PathBuf,
}

impl RemoteStateStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RemoteStorageError> {
        let path = path.as_ref().to_owned();
        ensure_state_file(&path)?;
        let connection = Connection::open(&path)
            .map_err(|source| RemoteStorageError::database("open", source))?;
        connection
            .execute_batch(SCHEMA)
            .map_err(|source| RemoteStorageError::database("initialize", source))?;
        Ok(Self { connection, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load_or_create(
        &mut self,
        device_name: impl Into<String>,
    ) -> Result<RemoteState, RemoteStorageError> {
        let metadata = self
            .connection
            .query_row(
                "SELECT format_version, length(payload) FROM remote_state WHERE singleton = 1",
                [],
                |row| Ok((row.get::<_, u16>(0)?, row.get::<_, usize>(1)?)),
            )
            .optional()
            .map_err(|source| RemoteStorageError::database("load", source))?;
        if let Some((version, byte_len)) = metadata {
            if version != STATE_FORMAT_VERSION {
                return Err(RemoteStorageError::UnsupportedVersion(version));
            }
            if byte_len > MAX_STATE_BYTES {
                return Err(RemoteStorageError::StateTooLarge);
            }
            let payload = self
                .connection
                .query_row(
                    "SELECT payload FROM remote_state WHERE singleton = 1",
                    [],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .map_err(|source| RemoteStorageError::database("read", source))?;
            return RemoteState::decode(&payload);
        }

        let state = RemoteState::new(DeviceIdentity::generate(device_name.into())?);
        self.save(&state)?;
        Ok(state)
    }

    pub fn save(&mut self, state: &RemoteState) -> Result<(), RemoteStorageError> {
        let payload = state.encode()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|source| RemoteStorageError::database("begin save", source))?;
        transaction
            .execute(
                "INSERT INTO remote_state (singleton, format_version, payload)
                 VALUES (1, ?1, ?2)
                 ON CONFLICT (singleton) DO UPDATE SET
                     format_version = excluded.format_version,
                     payload = excluded.payload",
                params![STATE_FORMAT_VERSION, payload],
            )
            .map_err(|source| RemoteStorageError::database("save", source))?;
        transaction
            .commit()
            .map_err(|source| RemoteStorageError::database("commit save", source))
    }

    /// Durably records a command ID before the caller performs its side effect.
    pub fn begin_command(
        &mut self,
        state: &mut RemoteState,
        device_id: DeviceId,
        command: &SteeringCommand,
    ) -> Result<CommandDisposition, RemoteStorageError> {
        let previous = state.ledger.clone();
        let disposition = state.begin_command(device_id, command)?;
        if disposition == CommandDisposition::Execute
            && let Err(error) = self.save(state)
        {
            state.ledger = previous;
            return Err(error);
        }
        Ok(disposition)
    }

    pub fn pair_device(
        &mut self,
        state: &mut RemoteState,
        device: PairedDevice,
    ) -> Result<(), RemoteStorageError> {
        let previous = state.registry.clone();
        state.registry.pair(device);
        if let Err(error) = self.save(state) {
            state.registry = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn revoke_device(
        &mut self,
        state: &mut RemoteState,
        device_id: DeviceId,
    ) -> Result<bool, RemoteStorageError> {
        let previous = state.registry.clone();
        let previous_sessions = state.sessions.clone();
        let existed = state.registry.revoke(device_id);
        state
            .sessions
            .retain(|_, checkpoint| checkpoint.peer_id != device_id);
        if let Err(error) = self.save(state) {
            state.registry = previous;
            state.sessions = previous_sessions;
            return Err(error);
        }
        Ok(existed)
    }

    pub fn update_grant(
        &mut self,
        state: &mut RemoteState,
        device_id: DeviceId,
        grant: PeerGrant,
    ) -> Result<(), RemoteStorageError> {
        let previous = state.registry.clone();
        let device = state
            .registry
            .device_mut(device_id)
            .ok_or(RemoteError::UnknownDevice)?;
        device.set_grant(grant);
        if let Err(error) = self.save(state) {
            state.registry = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Persists the terminal receipt before it is acknowledged to the peer.
    pub fn complete_command(
        &mut self,
        state: &mut RemoteState,
        command_id: CommandId,
        outcome: CommandOutcome,
    ) -> Result<CommandReceipt, RemoteStorageError> {
        let previous = state.ledger.clone();
        let receipt = state.complete_command(command_id, outcome)?;
        if let Err(error) = self.save(state) {
            state.ledger = previous;
            return Err(error);
        }
        Ok(receipt)
    }

    pub fn save_checkpoint(
        &mut self,
        state: &mut RemoteState,
        checkpoint: SessionCheckpoint,
    ) -> Result<(), RemoteStorageError> {
        let previous = state.sessions.clone();
        state.put_checkpoint(checkpoint)?;
        if let Err(error) = self.save(state) {
            state.sessions = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn start_session(
        &mut self,
        state: &mut RemoteState,
        peer_id: DeviceId,
    ) -> Result<super::RemoteSession, RemoteStorageError> {
        let peer = state
            .registry
            .device(peer_id)
            .ok_or(RemoteError::UnknownDevice)?;
        let session = loop {
            let candidate = super::RemoteSession::with_random_id(peer)?;
            if !state
                .sessions
                .contains_key(&candidate.checkpoint().session_id)
            {
                break candidate;
            }
        };
        self.save_checkpoint(state, session.checkpoint())?;
        Ok(session)
    }

    pub fn accept_session(
        &mut self,
        state: &mut RemoteState,
        peer_id: DeviceId,
        session_id: SessionId,
    ) -> Result<super::RemoteSession, RemoteStorageError> {
        if state.sessions.contains_key(&session_id) {
            return Err(RemoteError::InvalidPacket("session identifier was already used").into());
        }
        let peer = state
            .registry
            .device(peer_id)
            .ok_or(RemoteError::UnknownDevice)?;
        let session = super::RemoteSession::new(peer, session_id);
        self.save_checkpoint(state, session.checkpoint())?;
        Ok(session)
    }

    pub fn resume_session(
        &self,
        state: &RemoteState,
        session_id: SessionId,
    ) -> Option<super::RemoteSession> {
        state
            .checkpoint(session_id)
            .cloned()
            .map(super::RemoteSession::from_checkpoint)
    }

    pub fn encrypt_event(
        &mut self,
        state: &mut RemoteState,
        session: &mut RemoteSession,
        delta: StreamDelta,
    ) -> Result<EncryptedPacket, RemoteStorageError> {
        let previous = session.checkpoint();
        let result = session.send_event(&state.registry, delta);
        self.persist_session_result(state, session, previous, result)
    }

    pub fn encrypt_command(
        &mut self,
        state: &mut RemoteState,
        session: &mut RemoteSession,
        command: SteeringCommand,
    ) -> Result<EncryptedPacket, RemoteStorageError> {
        self.ensure_session_peer(state, session)?;
        let previous = session.checkpoint();
        let result = session.send_command(command);
        self.persist_session_result(state, session, previous, result)
    }

    pub fn encrypt_receipt(
        &mut self,
        state: &mut RemoteState,
        session: &mut RemoteSession,
        receipt: CommandReceipt,
    ) -> Result<EncryptedPacket, RemoteStorageError> {
        self.ensure_session_peer(state, session)?;
        let previous = session.checkpoint();
        let result = session.send_receipt(receipt);
        self.persist_session_result(state, session, previous, result)
    }

    pub fn encrypt_diagnostic(
        &mut self,
        state: &mut RemoteState,
        session: &mut RemoteSession,
        diagnostic: ConnectionDiagnostic,
    ) -> Result<EncryptedPacket, RemoteStorageError> {
        self.ensure_session_peer(state, session)?;
        let previous = session.checkpoint();
        let result = session.send_diagnostic(diagnostic);
        self.persist_session_result(state, session, previous, result)
    }

    pub fn encrypt_acknowledgement(
        &mut self,
        state: &mut RemoteState,
        session: &mut RemoteSession,
    ) -> Result<EncryptedPacket, RemoteStorageError> {
        self.ensure_session_peer(state, session)?;
        let previous = session.checkpoint();
        let result = session.acknowledgement();
        self.persist_session_result(state, session, previous, result)
    }

    pub fn receive_packet(
        &mut self,
        state: &mut RemoteState,
        session: &mut RemoteSession,
        packet: &EncryptedPacket,
    ) -> Result<InboundPacket, RemoteStorageError> {
        let previous = session.checkpoint();
        let result = session.receive(&state.registry, packet);
        self.persist_session_result(state, session, previous, result)
    }

    pub fn commit_delivery(
        &mut self,
        state: &mut RemoteState,
        session: &mut RemoteSession,
        sequence: u64,
    ) -> Result<(), RemoteStorageError> {
        self.ensure_session_peer(state, session)?;
        let previous = session.checkpoint();
        let result = session.commit_received(sequence);
        self.persist_session_result(state, session, previous, result)
    }

    pub fn reconnect_packets(
        &self,
        state: &RemoteState,
        session: &RemoteSession,
    ) -> Result<Vec<EncryptedPacket>, RemoteStorageError> {
        self.ensure_session_peer(state, session)?;
        Ok(session.reconnect())
    }

    fn ensure_session_peer(
        &self,
        state: &RemoteState,
        session: &RemoteSession,
    ) -> Result<(), RemoteStorageError> {
        state
            .registry
            .ensure_active(session.checkpoint().peer_id)
            .map_err(Into::into)
    }

    fn persist_session_result<T>(
        &mut self,
        state: &mut RemoteState,
        session: &mut RemoteSession,
        previous: SessionCheckpoint,
        result: Result<T, RemoteError>,
    ) -> Result<T, RemoteStorageError> {
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                *session = RemoteSession::from_checkpoint(previous);
                return Err(error.into());
            }
        };
        if let Err(error) = self.save_checkpoint(state, session.checkpoint()) {
            *session = RemoteSession::from_checkpoint(previous);
            return Err(error);
        }
        Ok(value)
    }
}

fn ensure_state_file(path: &Path) -> Result<(), RemoteStorageError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    match options.open(path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(source) => return Err(RemoteStorageError::file("create", path, source)),
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| RemoteStorageError::file("inspect", path, source))?;
    if !metadata.file_type().is_file() {
        return Err(RemoteStorageError::UnsafeFileType(path.to_owned()));
    }
    verify_permissions(path)
}

#[cfg(unix)]
fn verify_permissions(path: &Path) -> Result<(), RemoteStorageError> {
    let mode = fs::metadata(path)
        .map_err(|source| RemoteStorageError::file("inspect", path, source))?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        Err(RemoteStorageError::InsecurePermissions(path.to_owned()))
    } else {
        Ok(())
    }
}

#[cfg(not(unix))]
fn verify_permissions(_path: &Path) -> Result<(), RemoteStorageError> {
    Ok(())
}

#[derive(Debug)]
pub enum RemoteStorageError {
    FileAccess {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Database {
        operation: &'static str,
        source: rusqlite::Error,
    },
    Encode(serde_json::Error),
    Decode(serde_json::Error),
    Protocol(RemoteError),
    UnsafeFileType(PathBuf),
    InsecurePermissions(PathBuf),
    UnsupportedVersion(u16),
    StateTooLarge,
    InvalidState(&'static str),
}

impl RemoteStorageError {
    fn file(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::FileAccess {
            operation,
            path: path.to_owned(),
            source,
        }
    }

    fn database(operation: &'static str, source: rusqlite::Error) -> Self {
        Self::Database { operation, source }
    }
}

impl From<RemoteError> for RemoteStorageError {
    fn from(error: RemoteError) -> Self {
        Self::Protocol(error)
    }
}

impl Display for RemoteStorageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileAccess {
                operation, path, ..
            } => {
                write!(
                    formatter,
                    "could not {operation} remote state file {}",
                    path.display()
                )
            }
            Self::Database { operation, .. } => {
                write!(formatter, "remote state database {operation} failed")
            }
            Self::Encode(_) => formatter.write_str("could not encode remote recovery state"),
            Self::Decode(_) => formatter.write_str("remote recovery state is invalid"),
            Self::Protocol(error) => error.fmt(formatter),
            Self::UnsafeFileType(path) => {
                write!(
                    formatter,
                    "remote state path is not a regular file: {}",
                    path.display()
                )
            }
            Self::InsecurePermissions(path) => write!(
                formatter,
                "remote state file must not be accessible by other users: {}",
                path.display()
            ),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "remote state format {version} is unsupported")
            }
            Self::StateTooLarge => formatter.write_str("remote recovery state exceeds 64 MiB"),
            Self::InvalidState(reason) => {
                write!(formatter, "remote recovery state is invalid: {reason}")
            }
        }
    }
}

impl Error for RemoteStorageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::FileAccess { source, .. } => Some(source),
            Self::Database { source, .. } => Some(source),
            Self::Encode(source) | Self::Decode(source) => Some(source),
            Self::Protocol(source) => Some(source),
            _ => None,
        }
    }
}
