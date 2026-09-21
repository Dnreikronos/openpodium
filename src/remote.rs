//! End-to-end encrypted, capability-scoped remote access.

mod coordinator;
mod crypto;
mod protocol;
mod session;
mod storage;

pub use coordinator::{CommandHandling, SteeringHandler, handle_command};
pub use crypto::{
    DeviceIdentity, PairingConfirmation, PairingInvitation, PairingRequest, PairingReview,
    PendingClientPairing, PendingPairing,
};
pub use protocol::{
    CanvasDelta, Capability, ChatDelta, CommandId, CommandOutcome, CommandReceipt,
    ConnectionDiagnostic, DeviceId, DiagnosticState, EventId, NotificationDelta, PairingId,
    PeerGrant, RemotePayload, SessionId, SteeringAction, SteeringCommand, StreamDelta, TaskDelta,
};
pub use session::{
    CommandDisposition, CommandLedger, DeviceRegistry, EncryptedPacket, InboundPacket,
    PacketHeader, PairedDevice, RemoteError, RemoteSession, SessionCheckpoint,
};
pub use storage::{REMOTE_STATE_FILE_NAME, RemoteState, RemoteStateStore, RemoteStorageError};

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_PACKET_BYTES: usize = 1024 * 1024;

#[cfg(test)]
mod tests;
