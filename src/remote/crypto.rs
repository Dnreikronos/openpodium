use std::fmt::{self, Debug, Formatter};

use base64::Engine;
use serde::{Deserialize, Serialize};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

use super::protocol::{DeviceId, PairingId, PeerGrant};
use super::session::{PairedDevice, RemoteError};

const PAIRING_CODE_PREFIX: &str = "openpodium-pair-v1:";
const MAX_PAIRING_CODE_BYTES: usize = 4096;

pub struct DeviceIdentity {
    pub(crate) id: DeviceId,
    pub(crate) name: String,
    pub(crate) secret: StaticSecret,
    pub(crate) public_key: [u8; 32],
}

impl DeviceIdentity {
    pub fn generate(name: impl Into<String>) -> Result<Self, RemoteError> {
        let name = name.into();
        validate_device_name(&name)?;
        let secret = StaticSecret::from(random_bytes()?);
        let public_key = PublicKey::from(&secret).to_bytes();
        Ok(Self {
            id: DeviceId::from_bytes(random_bytes()?),
            name,
            secret,
            public_key,
        })
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

    pub(crate) fn restore(
        id: DeviceId,
        name: String,
        secret_bytes: [u8; 32],
    ) -> Result<Self, RemoteError> {
        validate_device_name(&name)?;
        let secret = StaticSecret::from(secret_bytes);
        let public_key = PublicKey::from(&secret).to_bytes();
        Ok(Self {
            id,
            name,
            secret,
            public_key,
        })
    }

    pub(crate) fn secret_bytes(&self) -> [u8; 32] {
        self.secret.to_bytes()
    }
}

impl Debug for DeviceIdentity {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceIdentity")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("secret", &"[REDACTED]")
            .field("public_key", &self.public_key)
            .finish()
    }
}

#[derive(Clone)]
pub struct PairingInvitation {
    pairing_id: PairingId,
    host_id: DeviceId,
    host_name: String,
    host_public_key: [u8; 32],
    secret: Zeroizing<[u8; 32]>,
    expires_at_millis: i64,
}

impl PairingInvitation {
    pub fn to_code(&self) -> Result<String, RemoteError> {
        let wire = PairingInvitationWire {
            pairing_id: self.pairing_id,
            host_id: self.host_id,
            host_name: self.host_name.clone(),
            host_public_key: self.host_public_key,
            secret: *self.secret,
            expires_at_millis: self.expires_at_millis,
        };
        let encoded = serde_json::to_vec(&wire).map_err(RemoteError::Encode)?;
        Ok(format!(
            "{PAIRING_CODE_PREFIX}{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(encoded)
        ))
    }

    pub fn from_code(code: &str) -> Result<Self, RemoteError> {
        if code.len() > MAX_PAIRING_CODE_BYTES {
            return Err(RemoteError::InvalidPairing("pairing code is too large"));
        }
        let encoded = code
            .strip_prefix(PAIRING_CODE_PREFIX)
            .ok_or(RemoteError::InvalidPairing("unrecognized pairing code"))?;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| RemoteError::InvalidPairing("pairing code is not valid base64"))?;
        let wire: PairingInvitationWire =
            serde_json::from_slice(&bytes).map_err(RemoteError::Decode)?;
        validate_device_name(&wire.host_name)?;
        Ok(Self {
            pairing_id: wire.pairing_id,
            host_id: wire.host_id,
            host_name: wire.host_name,
            host_public_key: wire.host_public_key,
            secret: Zeroizing::new(wire.secret),
            expires_at_millis: wire.expires_at_millis,
        })
    }

    pub const fn pairing_id(&self) -> PairingId {
        self.pairing_id
    }

    pub const fn expires_at_millis(&self) -> i64 {
        self.expires_at_millis
    }
}

impl Debug for PairingInvitation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairingInvitation")
            .field("pairing_id", &self.pairing_id)
            .field("host_id", &self.host_id)
            .field("host_name", &self.host_name)
            .field("host_public_key", &self.host_public_key)
            .field("secret", &"[REDACTED]")
            .field("expires_at_millis", &self.expires_at_millis)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairingInvitationWire {
    pairing_id: PairingId,
    host_id: DeviceId,
    host_name: String,
    host_public_key: [u8; 32],
    secret: [u8; 32],
    expires_at_millis: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingRequest {
    pairing_id: PairingId,
    device_id: DeviceId,
    device_name: String,
    public_key: [u8; 32],
    proof: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingReview {
    pub device_id: DeviceId,
    pub device_name: String,
    pub grant: PeerGrant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingConfirmation {
    pairing_id: PairingId,
    grant: PeerGrant,
    host_proof: [u8; 32],
}

pub struct PendingPairing<'a> {
    host: &'a DeviceIdentity,
    invitation: PairingInvitation,
    grant: PeerGrant,
    reviewed: Option<(PairingRequest, SecretKey)>,
}

impl<'a> PendingPairing<'a> {
    pub fn start(
        host: &'a DeviceIdentity,
        grant: PeerGrant,
        now_millis: i64,
        lifetime_millis: i64,
    ) -> Result<Self, RemoteError> {
        if lifetime_millis <= 0 {
            return Err(RemoteError::InvalidPairing(
                "pairing lifetime must be positive",
            ));
        }
        let expires_at_millis = now_millis
            .checked_add(lifetime_millis)
            .ok_or(RemoteError::InvalidPairing("pairing expiry overflow"))?;
        Ok(Self {
            host,
            invitation: PairingInvitation {
                pairing_id: PairingId::from_bytes(random_bytes()?),
                host_id: host.id,
                host_name: host.name.clone(),
                host_public_key: host.public_key,
                secret: Zeroizing::new(random_bytes()?),
                expires_at_millis,
            },
            grant,
            reviewed: None,
        })
    }

    pub fn invitation(&self) -> PairingInvitation {
        self.invitation.clone()
    }

    pub fn review(
        &mut self,
        request: PairingRequest,
        now_millis: i64,
    ) -> Result<PairingReview, RemoteError> {
        if self.reviewed.is_some() {
            return Err(RemoteError::InvalidPairing(
                "pairing request was already reviewed",
            ));
        }
        if now_millis > self.invitation.expires_at_millis {
            return Err(RemoteError::PairingExpired);
        }
        if request.pairing_id != self.invitation.pairing_id {
            return Err(RemoteError::InvalidPairing("pairing identifier mismatch"));
        }
        validate_device_name(&request.device_name)?;
        let root = derive_root(
            &self.host.secret,
            request.public_key,
            &self.invitation,
            request.device_id,
        )?;
        let expected = transcript_proof(&root, b"client", &self.invitation, &request);
        if !constant_time_eq(&expected, &request.proof) {
            return Err(RemoteError::InvalidPairing("client proof is invalid"));
        }
        let review = PairingReview {
            device_id: request.device_id,
            device_name: request.device_name.clone(),
            grant: self.grant.clone(),
        };
        self.reviewed = Some((request, root));
        Ok(review)
    }

    /// This method is the explicit local approval boundary.
    pub fn confirm(mut self) -> Result<(PairedDevice, PairingConfirmation), RemoteError> {
        let (request, root) = self
            .reviewed
            .take()
            .ok_or(RemoteError::PairingNotReviewed)?;
        let mut confirmation = PairingConfirmation {
            pairing_id: self.invitation.pairing_id,
            grant: self.grant.clone(),
            host_proof: [0; 32],
        };
        confirmation.host_proof =
            host_confirmation_proof(&root, &self.invitation, &request, &confirmation.grant)?;
        let device = PairedDevice::new(
            self.host.id,
            request.device_id,
            request.device_name,
            request.public_key,
            root,
            self.grant,
        );
        Ok((device, confirmation))
    }
}

pub struct PendingClientPairing {
    invitation: PairingInvitation,
    request: PairingRequest,
    root: SecretKey,
}

impl PendingClientPairing {
    pub fn accept(
        identity: &DeviceIdentity,
        invitation: PairingInvitation,
        now_millis: i64,
    ) -> Result<Self, RemoteError> {
        if now_millis > invitation.expires_at_millis {
            return Err(RemoteError::PairingExpired);
        }
        let root = derive_root(
            &identity.secret,
            invitation.host_public_key,
            &invitation,
            identity.id,
        )?;
        let mut request = PairingRequest {
            pairing_id: invitation.pairing_id,
            device_id: identity.id,
            device_name: identity.name.clone(),
            public_key: identity.public_key,
            proof: [0; 32],
        };
        request.proof = transcript_proof(&root, b"client", &invitation, &request);
        Ok(Self {
            invitation,
            request,
            root,
        })
    }

    pub fn request(&self) -> PairingRequest {
        self.request.clone()
    }

    pub fn finish(self, confirmation: PairingConfirmation) -> Result<PairedDevice, RemoteError> {
        if confirmation.pairing_id != self.invitation.pairing_id {
            return Err(RemoteError::InvalidPairing("pairing identifier mismatch"));
        }
        let expected = host_confirmation_proof(
            &self.root,
            &self.invitation,
            &self.request,
            &confirmation.grant,
        )?;
        if !constant_time_eq(&expected, &confirmation.host_proof) {
            return Err(RemoteError::InvalidPairing("host proof is invalid"));
        }
        Ok(PairedDevice::new(
            self.request.device_id,
            self.invitation.host_id,
            self.invitation.host_name,
            self.invitation.host_public_key,
            self.root,
            confirmation.grant,
        ))
    }
}

pub(crate) struct SecretKey(Zeroizing<[u8; 32]>);

impl SecretKey {
    pub(crate) fn new(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub(crate) fn bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub(crate) fn copy_bytes(&self) -> [u8; 32] {
        *self.0
    }
}

impl Clone for SecretKey {
    fn clone(&self) -> Self {
        Self::new(*self.0)
    }
}

impl Debug for SecretKey {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

pub(crate) fn derive_session_key(
    root: &SecretKey,
    session_id: &[u8; 16],
    sender: DeviceId,
    receiver: DeviceId,
) -> SecretKey {
    let mut input = Vec::with_capacity(16 + 16 + 16);
    input.extend_from_slice(session_id);
    input.extend_from_slice(sender.as_bytes());
    input.extend_from_slice(receiver.as_bytes());
    SecretKey::new(blake3::derive_key(
        "openpodium remote session direction v1",
        &[root.bytes().as_slice(), input.as_slice()].concat(),
    ))
}

pub(crate) fn random_bytes<const N: usize>() -> Result<[u8; N], RemoteError> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(RemoteError::Random)?;
    Ok(bytes)
}

fn derive_root(
    local_secret: &StaticSecret,
    remote_public: [u8; 32],
    invitation: &PairingInvitation,
    remote_id: DeviceId,
) -> Result<SecretKey, RemoteError> {
    let shared = local_secret.diffie_hellman(&PublicKey::from(remote_public));
    if shared.as_bytes().iter().all(|byte| *byte == 0) {
        return Err(RemoteError::InvalidPairing("invalid device public key"));
    }
    let mut material = Vec::with_capacity(32 + 32 + 16 * 3);
    material.extend_from_slice(shared.as_bytes());
    material.extend_from_slice(invitation.secret.as_slice());
    material.extend_from_slice(invitation.pairing_id.as_bytes());
    material.extend_from_slice(invitation.host_id.as_bytes());
    material.extend_from_slice(remote_id.as_bytes());
    Ok(SecretKey::new(blake3::derive_key(
        "openpodium remote pairing root v1",
        &material,
    )))
}

fn transcript_proof(
    root: &SecretKey,
    role: &[u8],
    invitation: &PairingInvitation,
    request: &PairingRequest,
) -> [u8; 32] {
    let mut transcript = Vec::new();
    transcript.extend_from_slice(role);
    transcript.extend_from_slice(invitation.pairing_id.as_bytes());
    transcript.extend_from_slice(invitation.host_id.as_bytes());
    transcript.extend_from_slice(&invitation.host_public_key);
    transcript.extend_from_slice(request.device_id.as_bytes());
    transcript.extend_from_slice(&request.public_key);
    transcript.extend_from_slice(invitation.host_name.as_bytes());
    transcript.extend_from_slice(request.device_name.as_bytes());
    *blake3::keyed_hash(root.bytes(), &transcript).as_bytes()
}

fn host_confirmation_proof(
    root: &SecretKey,
    invitation: &PairingInvitation,
    request: &PairingRequest,
    grant: &PeerGrant,
) -> Result<[u8; 32], RemoteError> {
    let mut transcript = transcript_proof(root, b"host", invitation, request).to_vec();
    transcript.extend_from_slice(&serde_json::to_vec(grant).map_err(RemoteError::Encode)?);
    Ok(*blake3::keyed_hash(root.bytes(), &transcript).as_bytes())
}

fn validate_device_name(name: &str) -> Result<(), RemoteError> {
    if name.trim().is_empty() || name.chars().count() > 128 {
        Err(RemoteError::InvalidPairing(
            "device name must contain 1 to 128 characters",
        ))
    } else {
        Ok(())
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        let left = left.get(index).copied().unwrap_or(0);
        let right = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(left ^ right);
    }
    difference == 0
}
