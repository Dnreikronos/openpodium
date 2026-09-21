use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::crypto::{SecretKey, derive_session_key, random_bytes};
use super::protocol::{
    Capability, CommandId, CommandOutcome, CommandReceipt, DeviceId, PeerGrant, RemotePayload,
    SessionId, SteeringCommand, StreamDelta,
};
use super::{MAX_PACKET_BYTES, PROTOCOL_VERSION};

pub(crate) const MAX_OUTBOX_PACKETS: usize = 64;
const MAX_PACKET_WIRE_BYTES: usize = (MAX_PACKET_BYTES + 16) * 4 / 3 + 4096;

#[derive(Clone)]
pub struct PairedDevice {
    pub(crate) local_id: DeviceId,
    pub(crate) id: DeviceId,
    pub(crate) name: String,
    pub(crate) public_key: [u8; 32],
    pub(crate) root: SecretKey,
    pub(crate) grant: PeerGrant,
}

impl PairedDevice {
    pub(crate) fn new(
        local_id: DeviceId,
        id: DeviceId,
        name: String,
        public_key: [u8; 32],
        root: SecretKey,
        grant: PeerGrant,
    ) -> Self {
        Self {
            local_id,
            id,
            name,
            public_key,
            root,
            grant,
        }
    }

    pub const fn id(&self) -> DeviceId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn public_key(&self) -> &[u8; 32] {
        &self.public_key
    }

    pub const fn grant(&self) -> &PeerGrant {
        &self.grant
    }

    pub fn set_grant(&mut self, grant: PeerGrant) {
        self.grant = grant;
    }
}

impl fmt::Debug for PairedDevice {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairedDevice")
            .field("local_id", &self.local_id)
            .field("id", &self.id)
            .field("name", &self.name)
            .field("public_key", &self.public_key)
            .field("root", &"[REDACTED]")
            .field("grant", &self.grant)
            .finish()
    }
}

#[derive(Debug, Default, Clone)]
pub struct DeviceRegistry {
    pub(crate) devices: BTreeMap<DeviceId, PairedDevice>,
    pub(crate) revoked: BTreeSet<DeviceId>,
}

impl DeviceRegistry {
    pub fn pair(&mut self, device: PairedDevice) {
        self.revoked.remove(&device.id);
        self.devices.insert(device.id, device);
    }

    pub fn revoke(&mut self, device_id: DeviceId) -> bool {
        let existed = self.devices.remove(&device_id).is_some();
        self.revoked.insert(device_id);
        existed
    }

    pub fn device(&self, device_id: DeviceId) -> Option<&PairedDevice> {
        self.devices.get(&device_id)
    }

    pub fn device_mut(&mut self, device_id: DeviceId) -> Option<&mut PairedDevice> {
        self.devices.get_mut(&device_id)
    }

    pub fn authorize(
        &self,
        device_id: DeviceId,
        workspace_id: u64,
        capability: Capability,
    ) -> Result<(), RemoteError> {
        if self.revoked.contains(&device_id) {
            return Err(RemoteError::RevokedDevice);
        }
        let device = self
            .devices
            .get(&device_id)
            .ok_or(RemoteError::UnknownDevice)?;
        if device.grant.allows(workspace_id, capability) {
            Ok(())
        } else {
            Err(RemoteError::CapabilityDenied)
        }
    }

    pub(crate) fn ensure_active(&self, device_id: DeviceId) -> Result<(), RemoteError> {
        if self.revoked.contains(&device_id) {
            Err(RemoteError::RevokedDevice)
        } else if self.devices.contains_key(&device_id) {
            Ok(())
        } else {
            Err(RemoteError::UnknownDevice)
        }
    }

    pub fn authorize_delta(
        &self,
        device_id: DeviceId,
        delta: &StreamDelta,
    ) -> Result<(), RemoteError> {
        self.authorize(device_id, delta.workspace_id(), delta.required_capability())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PacketHeader {
    pub version: u16,
    pub session_id: SessionId,
    pub sender_id: DeviceId,
    pub nonce_counter: u64,
    pub delivery_sequence: Option<u64>,
    pub acknowledged_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncryptedPacket {
    pub header: PacketHeader,
    #[serde(with = "base64_bytes")]
    pub ciphertext: Vec<u8>,
}

impl EncryptedPacket {
    pub fn encode(&self) -> Result<Vec<u8>, RemoteError> {
        if self.ciphertext.len() > MAX_PACKET_BYTES + 16 {
            return Err(RemoteError::PacketTooLarge);
        }
        serde_json::to_vec(self).map_err(RemoteError::Encode)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, RemoteError> {
        if bytes.len() > MAX_PACKET_WIRE_BYTES {
            return Err(RemoteError::PacketTooLarge);
        }
        let packet: Self = serde_json::from_slice(bytes).map_err(RemoteError::Decode)?;
        if packet.ciphertext.len() > MAX_PACKET_BYTES + 16 {
            return Err(RemoteError::PacketTooLarge);
        }
        if packet.header.version != PROTOCOL_VERSION || packet.header.nonce_counter == 0 {
            return Err(RemoteError::InvalidPacket(
                "packet version or nonce counter is invalid",
            ));
        }
        Ok(packet)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboundPacket {
    Delivery {
        sequence: u64,
        payload: RemotePayload,
    },
    Duplicate {
        acknowledged_sequence: u64,
    },
    Acknowledgement {
        acknowledged_sequence: u64,
    },
    Gap {
        expected: u64,
        received: u64,
    },
}

#[derive(Clone)]
pub struct SessionCheckpoint {
    pub(crate) session_id: SessionId,
    pub(crate) local_id: DeviceId,
    pub(crate) peer_id: DeviceId,
    pub(crate) root: SecretKey,
    pub(crate) next_nonce: u64,
    pub(crate) next_delivery: u64,
    pub(crate) committed_inbound: u64,
    pub(crate) outbox: BTreeMap<u64, EncryptedPacket>,
}

impl fmt::Debug for SessionCheckpoint {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionCheckpoint")
            .field("session_id", &self.session_id)
            .field("local_id", &self.local_id)
            .field("peer_id", &self.peer_id)
            .field("root", &"[REDACTED]")
            .field("next_nonce", &self.next_nonce)
            .field("next_delivery", &self.next_delivery)
            .field("committed_inbound", &self.committed_inbound)
            .field("outbox_len", &self.outbox.len())
            .finish()
    }
}

pub struct RemoteSession {
    checkpoint: SessionCheckpoint,
    send_key: SecretKey,
    receive_key: SecretKey,
    staged: Option<(u64, RemotePayload)>,
    highest_control_nonce: u64,
}

impl RemoteSession {
    pub(crate) fn new(peer: &PairedDevice, session_id: SessionId) -> Self {
        Self::from_checkpoint(SessionCheckpoint {
            session_id,
            local_id: peer.local_id,
            peer_id: peer.id,
            root: peer.root.clone(),
            next_nonce: 1,
            next_delivery: 1,
            committed_inbound: 0,
            outbox: BTreeMap::new(),
        })
    }

    pub(crate) fn with_random_id(peer: &PairedDevice) -> Result<Self, RemoteError> {
        Ok(Self::new(peer, SessionId::from_bytes(random_bytes()?)))
    }

    pub(crate) fn from_checkpoint(checkpoint: SessionCheckpoint) -> Self {
        let send_key = derive_session_key(
            &checkpoint.root,
            checkpoint.session_id.as_bytes(),
            checkpoint.local_id,
            checkpoint.peer_id,
        );
        let receive_key = derive_session_key(
            &checkpoint.root,
            checkpoint.session_id.as_bytes(),
            checkpoint.peer_id,
            checkpoint.local_id,
        );
        Self {
            checkpoint,
            send_key,
            receive_key,
            staged: None,
            highest_control_nonce: 0,
        }
    }

    pub fn checkpoint(&self) -> SessionCheckpoint {
        self.checkpoint.clone()
    }

    pub(crate) fn send_event(
        &mut self,
        registry: &DeviceRegistry,
        delta: StreamDelta,
    ) -> Result<EncryptedPacket, RemoteError> {
        registry.authorize_delta(self.checkpoint.peer_id, &delta)?;
        self.send_payload(&RemotePayload::Event(delta))
    }

    pub(crate) fn send_command(
        &mut self,
        command: SteeringCommand,
    ) -> Result<EncryptedPacket, RemoteError> {
        self.send_payload(&RemotePayload::Command(command))
    }

    pub(crate) fn send_receipt(
        &mut self,
        receipt: CommandReceipt,
    ) -> Result<EncryptedPacket, RemoteError> {
        self.send_payload(&RemotePayload::Receipt(receipt))
    }

    pub(crate) fn send_diagnostic(
        &mut self,
        diagnostic: super::protocol::ConnectionDiagnostic,
    ) -> Result<EncryptedPacket, RemoteError> {
        self.send_payload(&RemotePayload::Diagnostic(diagnostic))
    }

    fn send_payload(&mut self, payload: &RemotePayload) -> Result<EncryptedPacket, RemoteError> {
        payload.validate().map_err(RemoteError::InvalidPayload)?;
        if self.checkpoint.outbox.len() >= MAX_OUTBOX_PACKETS {
            return Err(RemoteError::OutboxFull);
        }
        let sequence = self.checkpoint.next_delivery;
        self.checkpoint.next_delivery = sequence
            .checked_add(1)
            .ok_or(RemoteError::SequenceExhausted)?;
        let packet = self.encrypt(Some(sequence), payload)?;
        self.checkpoint.outbox.insert(sequence, packet.clone());
        Ok(packet)
    }

    pub(crate) fn acknowledgement(&mut self) -> Result<EncryptedPacket, RemoteError> {
        self.encrypt(
            None,
            &RemotePayload::Diagnostic(super::protocol::ConnectionDiagnostic {
                state: super::protocol::DiagnosticState::Connected,
                detail: String::new(),
            }),
        )
    }

    pub(crate) fn receive(
        &mut self,
        registry: &DeviceRegistry,
        packet: &EncryptedPacket,
    ) -> Result<InboundPacket, RemoteError> {
        registry.ensure_active(self.checkpoint.peer_id)?;
        self.validate_header(packet)?;
        let plaintext = decrypt(&self.receive_key, packet)?;
        self.acknowledge_outbox(packet.header.acknowledged_sequence);

        if packet.header.delivery_sequence.is_none() {
            if packet.header.nonce_counter <= self.highest_control_nonce {
                return Ok(InboundPacket::Duplicate {
                    acknowledged_sequence: self.checkpoint.committed_inbound,
                });
            }
            self.highest_control_nonce = packet.header.nonce_counter;
            return Ok(InboundPacket::Acknowledgement {
                acknowledged_sequence: self.checkpoint.committed_inbound,
            });
        }

        let sequence = packet
            .header
            .delivery_sequence
            .expect("delivery sequence was checked above");
        if sequence <= self.checkpoint.committed_inbound
            || self
                .staged
                .as_ref()
                .is_some_and(|(staged, _)| *staged == sequence)
        {
            return Ok(InboundPacket::Duplicate {
                acknowledged_sequence: self.checkpoint.committed_inbound,
            });
        }
        let expected = self
            .checkpoint
            .committed_inbound
            .checked_add(1)
            .ok_or(RemoteError::SequenceExhausted)?;
        if sequence != expected {
            return Ok(InboundPacket::Gap {
                expected,
                received: sequence,
            });
        }
        let payload: RemotePayload =
            serde_json::from_slice(&plaintext).map_err(RemoteError::Decode)?;
        payload.validate().map_err(RemoteError::InvalidPayload)?;
        self.staged = Some((sequence, payload.clone()));
        Ok(InboundPacket::Delivery { sequence, payload })
    }

    /// Call only after the delivered payload and any command receipt are durable.
    pub(crate) fn commit_received(&mut self, sequence: u64) -> Result<(), RemoteError> {
        match self.staged.take() {
            Some((staged, _)) if staged == sequence => {
                self.checkpoint.committed_inbound = sequence;
                Ok(())
            }
            Some(staged) => {
                self.staged = Some(staged);
                Err(RemoteError::UnexpectedCommit)
            }
            None => Err(RemoteError::UnexpectedCommit),
        }
    }

    pub(crate) fn reconnect(&self) -> Vec<EncryptedPacket> {
        self.checkpoint.outbox.values().cloned().collect()
    }

    pub const fn committed_inbound(&self) -> u64 {
        self.checkpoint.committed_inbound
    }

    pub fn pending_outbound(&self) -> usize {
        self.checkpoint.outbox.len()
    }

    fn encrypt(
        &mut self,
        delivery_sequence: Option<u64>,
        payload: &RemotePayload,
    ) -> Result<EncryptedPacket, RemoteError> {
        let plaintext = serde_json::to_vec(payload).map_err(RemoteError::Encode)?;
        if plaintext.len() > MAX_PACKET_BYTES {
            return Err(RemoteError::PacketTooLarge);
        }
        let nonce_counter = self.checkpoint.next_nonce;
        self.checkpoint.next_nonce = nonce_counter
            .checked_add(1)
            .ok_or(RemoteError::SequenceExhausted)?;
        let header = PacketHeader {
            version: PROTOCOL_VERSION,
            session_id: self.checkpoint.session_id,
            sender_id: self.checkpoint.local_id,
            nonce_counter,
            delivery_sequence,
            acknowledged_sequence: self.checkpoint.committed_inbound,
        };
        let associated_data = serde_json::to_vec(&header).map_err(RemoteError::Encode)?;
        let cipher = XChaCha20Poly1305::new(Key::from_slice(self.send_key.bytes()));
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce(&header)),
                Payload {
                    msg: &plaintext,
                    aad: &associated_data,
                },
            )
            .map_err(|_| RemoteError::CryptographicFailure)?;
        Ok(EncryptedPacket { header, ciphertext })
    }

    fn validate_header(&self, packet: &EncryptedPacket) -> Result<(), RemoteError> {
        if packet.ciphertext.len() > MAX_PACKET_BYTES + 16 {
            return Err(RemoteError::PacketTooLarge);
        }
        if packet.header.version != PROTOCOL_VERSION {
            return Err(RemoteError::ProtocolMismatch);
        }
        if packet.header.session_id != self.checkpoint.session_id
            || packet.header.sender_id != self.checkpoint.peer_id
        {
            return Err(RemoteError::WrongSession);
        }
        if packet.header.nonce_counter == 0 {
            return Err(RemoteError::InvalidPacket("nonce counter must be positive"));
        }
        if packet.header.acknowledged_sequence >= self.checkpoint.next_delivery {
            return Err(RemoteError::InvalidPacket(
                "acknowledgement exceeds the highest sent sequence",
            ));
        }
        Ok(())
    }

    fn acknowledge_outbox(&mut self, acknowledged: u64) {
        self.checkpoint
            .outbox
            .retain(|sequence, _| *sequence > acknowledged);
    }
}

fn decrypt(key: &SecretKey, packet: &EncryptedPacket) -> Result<Vec<u8>, RemoteError> {
    let associated_data = serde_json::to_vec(&packet.header).map_err(RemoteError::Encode)?;
    XChaCha20Poly1305::new(Key::from_slice(key.bytes()))
        .decrypt(
            XNonce::from_slice(&nonce(&packet.header)),
            Payload {
                msg: &packet.ciphertext,
                aad: &associated_data,
            },
        )
        .map_err(|_| RemoteError::CryptographicFailure)
}

fn nonce(header: &PacketHeader) -> [u8; 24] {
    let mut nonce = [0_u8; 24];
    nonce[..16].copy_from_slice(header.session_id.as_bytes());
    nonce[16..].copy_from_slice(&header.nonce_counter.to_be_bytes());
    nonce
}

mod base64_bytes {
    use super::*;

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        if encoded.len() > MAX_PACKET_WIRE_BYTES {
            return Err(serde::de::Error::custom(
                "ciphertext exceeds protocol limit",
            ));
        }
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum CommandRecord {
    Pending,
    Completed(CommandReceipt),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandDisposition {
    Execute,
    Pending,
    Completed(CommandReceipt),
}

#[derive(Debug, Default, Clone)]
pub struct CommandLedger {
    pub(crate) records: BTreeMap<CommandId, CommandRecord>,
}

impl CommandLedger {
    pub fn begin(
        &mut self,
        registry: &DeviceRegistry,
        device_id: DeviceId,
        command: &SteeringCommand,
    ) -> Result<CommandDisposition, RemoteError> {
        registry.authorize(
            device_id,
            command.workspace_id,
            command.action.required_capability(),
        )?;
        match self.records.get(&command.id) {
            Some(CommandRecord::Pending) => Ok(CommandDisposition::Pending),
            Some(CommandRecord::Completed(receipt)) => {
                Ok(CommandDisposition::Completed(receipt.clone()))
            }
            None => {
                self.records.insert(command.id, CommandRecord::Pending);
                Ok(CommandDisposition::Execute)
            }
        }
    }

    pub fn complete(
        &mut self,
        command_id: CommandId,
        outcome: CommandOutcome,
    ) -> Result<CommandReceipt, RemoteError> {
        let record = self
            .records
            .get_mut(&command_id)
            .ok_or(RemoteError::UnknownCommand)?;
        if let CommandRecord::Completed(receipt) = record {
            return Ok(receipt.clone());
        }
        let receipt = CommandReceipt {
            command_id,
            outcome,
        };
        *record = CommandRecord::Completed(receipt.clone());
        Ok(receipt)
    }
}

#[derive(Debug)]
pub enum RemoteError {
    Random(getrandom::Error),
    Encode(serde_json::Error),
    Decode(serde_json::Error),
    InvalidPairing(&'static str),
    PairingExpired,
    PairingNotReviewed,
    UnknownDevice,
    RevokedDevice,
    CapabilityDenied,
    PacketTooLarge,
    ProtocolMismatch,
    WrongSession,
    CryptographicFailure,
    InvalidPacket(&'static str),
    InvalidPayload(&'static str),
    OutboxFull,
    SequenceExhausted,
    UnexpectedCommit,
    UnknownCommand,
}

impl Display for RemoteError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Random(_) => formatter.write_str("secure randomness is unavailable"),
            Self::Encode(_) => formatter.write_str("could not encode remote protocol data"),
            Self::Decode(_) => formatter.write_str("remote protocol data is invalid"),
            Self::InvalidPairing(reason) => write!(formatter, "invalid pairing: {reason}"),
            Self::PairingExpired => formatter.write_str("pairing invitation expired"),
            Self::PairingNotReviewed => {
                formatter.write_str("pairing must be reviewed before local confirmation")
            }
            Self::UnknownDevice => formatter.write_str("remote device is not paired"),
            Self::RevokedDevice => formatter.write_str("remote device was revoked"),
            Self::CapabilityDenied => formatter.write_str("remote capability is not granted"),
            Self::PacketTooLarge => formatter.write_str("remote packet exceeds the size limit"),
            Self::ProtocolMismatch => formatter.write_str("remote protocol version is unsupported"),
            Self::WrongSession => formatter.write_str("remote packet belongs to another session"),
            Self::CryptographicFailure => {
                formatter.write_str("remote packet authentication failed")
            }
            Self::InvalidPacket(reason) => write!(formatter, "invalid remote packet: {reason}"),
            Self::InvalidPayload(reason) => write!(formatter, "invalid remote payload: {reason}"),
            Self::OutboxFull => formatter.write_str("remote outbox is full; reconnect is required"),
            Self::SequenceExhausted => formatter.write_str("remote sequence space is exhausted"),
            Self::UnexpectedCommit => formatter.write_str("no matching remote delivery is pending"),
            Self::UnknownCommand => formatter.write_str("remote command is not pending"),
        }
    }
}

impl Error for RemoteError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Random(source) => Some(source),
            Self::Encode(source) | Self::Decode(source) => Some(source),
            _ => None,
        }
    }
}
