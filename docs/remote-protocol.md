# Encrypted remote monitoring and steering

Status: Issue #25 protocol contract  
Updated: 2026-09-21

## Purpose and boundary

OpenPodium can pair another user-controlled device to observe selected local
workspace state and submit a narrow set of commands. The remote protocol is
transport agnostic: direct sockets, local discovery, and store-and-forward
relays carry the same opaque packets and are outside the trusted computing
base. A relay learns packet timing, size, device identifiers, and session
identifiers, but never receives a content-encryption key or plaintext project
content.

Remote access is not ambient account access. Every device is paired by an
explicit action on the local OpenPodium instance, receives an allowlist of
workspace and operation capabilities, and can be revoked locally without the
remote device's cooperation.

## Pairing protocol

1. The local user starts pairing and chooses workspaces and capabilities.
   OpenPodium creates a single-use 256-bit secret, a pairing identifier, and an
   expiry. The invitation contains those values plus the local device's X25519
   public key and is intended for an out-of-band QR or copy flow.
2. The remote device creates its own persistent X25519 identity, derives a
   pairing root from the static-static Diffie-Hellman result and the invitation
   secret, and sends its public identity plus a keyed transcript proof.
3. The local UI displays the requesting device name and grant. Nothing becomes
   trusted until the user confirms this pending request locally.
4. Confirmation returns the local transcript proof. Both peers then retain the
   other public identity and the derived root. Invitations are consumed on
   confirmation, rejection, or expiry and cannot be replayed.

The invitation secret authenticates the otherwise unauthenticated first
exchange. It must be transferred only by the explicit pairing surface and must
never be logged. A successful pairing produces a stable `DeviceId`; revocation
removes its grant and causes every packet from that device to be rejected.

## Session encryption

Each connection uses a fresh random 128-bit session identifier. Directional
keys are derived from the pairing root, both device identities, the session
identifier, and a direction label. Payloads use XChaCha20-Poly1305. The packet
header (protocol version, session, sender, sequence, and cumulative
acknowledgement) is authenticated as associated data.

The 192-bit nonce is the session identifier followed by the monotonically
increasing 64-bit sequence. A sender must never reset a sequence within a
session. Reconnecting either resumes the checkpointed session or creates a new
random session; it never reuses a session identifier with reset counters.
Packets are capped at 1 MiB before decryption and decoded with unknown fields
rejected.

## Incremental stream and reconnects

The encrypted payload is one of:

- a task lifecycle delta;
- an appended chat message;
- a notification with evidence references;
- a selected canvas-node delta;
- a `prompt`, `approve`, `cancel`, or `resume` command;
- a command receipt or connection diagnostic.

Every outbound payload stays in an ordered outbox until the peer acknowledges
its sequence. Receivers accept only the next sequence, expose the decrypted
payload to the application, and advance their durable acknowledgement only
after the application commits the corresponding event or command receipt.
Duplicates return the prior acknowledgement without redelivery; gaps request
retransmission from the first missing sequence. Reconnect sends the last
committed acknowledgement and retransmits the remaining outbox verbatim.
The outbox is capped at 64 packets; producers pause with a reconnect-required
diagnostic instead of allowing an offline peer to consume unbounded memory.

Steering commands also carry a random 128-bit command identifier. The local
command ledger durably records a pending marker before execution and persists
the completed receipt before acknowledging it, so retransmission never executes
the same command twice. A pending command recovered after a crash is surfaced
for local reconciliation rather than being executed again automatically.

## Authorization

Grants are the intersection of a workspace allowlist and these independent
capabilities:

- observe tasks;
- observe chat;
- observe notifications;
- observe selected canvas state;
- prompt an agent;
- approve a pending local action;
- cancel a task;
- resume a task.

Filtering happens before serialization and encryption. A remote request is
checked against the current local grant when received, not the grant cached at
pairing time. Revocation and grant reductions therefore take effect before the
next payload is accepted. Approval never bypasses the existing local approval
policy; it only answers an approval that the local instance already exposed to
that device.

## Recovery and diagnostics

Recovery state contains the session identifier, directional root, send and
receive counters, encrypted outbox, and command ledger. It is secret-bearing
state and uses the same restrictive storage policy as the local IPC secret.
After offline recovery, diagnostics distinguish connecting, connected,
reconnecting, offline, revoked, protocol mismatch, authorization failure, and
cryptographic failure without including project content or key material.

Clock time is diagnostic only. Sequence numbers, acknowledgements, invitation
expiry checked by the local clock, and durable command IDs determine protocol
behavior.
