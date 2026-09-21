use super::*;
use proptest::prelude::*;

fn grant(capabilities: impl IntoIterator<Item = Capability>) -> PeerGrant {
    PeerGrant::new([7], capabilities)
}

fn paired_devices() -> (DeviceIdentity, DeviceIdentity, PairedDevice, PairedDevice) {
    let host = DeviceIdentity::generate("Desktop").unwrap();
    let (client, client_on_host, host_on_client) = pair_with_host(&host);
    (host, client, client_on_host, host_on_client)
}

fn pair_with_host(host: &DeviceIdentity) -> (DeviceIdentity, PairedDevice, PairedDevice) {
    let client = DeviceIdentity::generate("Phone").unwrap();
    let permissions = grant([
        Capability::ObserveTasks,
        Capability::Prompt,
        Capability::Cancel,
    ]);
    let mut pending = PendingPairing::start(host, permissions.clone(), 1_000, 60_000).unwrap();
    let invitation =
        PairingInvitation::from_code(&pending.invitation().to_code().unwrap()).unwrap();
    let client_pending = PendingClientPairing::accept(&client, invitation, 1_001).unwrap();

    let review = pending.review(client_pending.request(), 1_002).unwrap();
    assert_eq!(review.device_id, client.id());
    let (client_on_host, confirmation) = pending.confirm().unwrap();
    let host_on_client = client_pending.finish(confirmation).unwrap();
    (client, client_on_host, host_on_client)
}

fn prompt(id: u8, body: &str) -> RemotePayload {
    RemotePayload::Command(SteeringCommand {
        id: CommandId::from_bytes([id; 16]),
        workspace_id: 7,
        action: SteeringAction::Prompt {
            agent_id: 9,
            body: body.to_owned(),
        },
    })
}

fn prompt_command(id: u8, body: &str) -> SteeringCommand {
    let RemotePayload::Command(command) = prompt(id, body) else {
        unreachable!();
    };
    command
}

#[test]
fn pairing_requires_review_and_explicit_local_confirmation() {
    let host = DeviceIdentity::generate("Desktop").unwrap();
    let client = DeviceIdentity::generate("Phone").unwrap();
    let permissions = grant([Capability::Prompt]);
    let pending = PendingPairing::start(&host, permissions, 10, 100).unwrap();
    let client_pending = PendingClientPairing::accept(&client, pending.invitation(), 11).unwrap();

    assert!(matches!(
        pending.confirm(),
        Err(RemoteError::PairingNotReviewed)
    ));

    let expired = PendingPairing::start(&host, grant([]), 10, 1).unwrap();
    assert!(matches!(
        PendingClientPairing::accept(&client, expired.invitation(), 12),
        Err(RemoteError::PairingExpired)
    ));

    drop(client_pending);
}

#[test]
fn pairing_confirmation_authenticates_the_local_grant() {
    let host = DeviceIdentity::generate("Desktop").unwrap();
    let client = DeviceIdentity::generate("Phone").unwrap();
    let permissions = grant([Capability::Prompt]);
    let mut pending = PendingPairing::start(&host, permissions, 10, 100).unwrap();
    let client_pending = PendingClientPairing::accept(&client, pending.invitation(), 11).unwrap();
    pending.review(client_pending.request(), 12).unwrap();
    let (_, confirmation) = pending.confirm().unwrap();

    let mut wire = serde_json::to_value(confirmation).unwrap();
    wire["grant"]["capabilities"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!("approve"));
    let tampered: PairingConfirmation = serde_json::from_value(wire).unwrap();
    assert!(matches!(
        client_pending.finish(tampered),
        Err(RemoteError::InvalidPairing("host proof is invalid"))
    ));
}

#[test]
fn relay_observes_ciphertext_and_tampering_is_rejected() {
    let (_, _, client_on_host, host_on_client) = paired_devices();
    let session_id = SessionId::from_bytes([3; 16]);
    let mut host_session = RemoteSession::new(&client_on_host, session_id);
    let mut client_session = RemoteSession::new(&host_on_client, session_id);
    let mut client_registry = DeviceRegistry::default();
    client_registry.pair(host_on_client);
    let packet = host_session
        .send_command(prompt_command(1, "secret project prompt"))
        .unwrap();

    let wire = serde_json::to_vec(&packet).unwrap();
    assert!(
        !wire
            .windows(b"secret project prompt".len())
            .any(|window| window == b"secret project prompt")
    );

    let mut tampered = packet.clone();
    tampered.ciphertext[0] ^= 1;
    assert!(matches!(
        client_session.receive(&client_registry, &tampered),
        Err(RemoteError::CryptographicFailure)
    ));
    assert!(matches!(
        client_session.receive(&client_registry, &packet).unwrap(),
        InboundPacket::Delivery { sequence: 1, .. }
    ));
    assert_eq!(
        EncryptedPacket::decode(&packet.encode().unwrap()).unwrap(),
        packet
    );
}

#[test]
fn current_grant_and_revocation_bound_remote_capabilities() {
    let (_, client, client_on_host, host_on_client) = paired_devices();
    let mut registry = DeviceRegistry::default();
    registry.pair(client_on_host.clone());
    let command = SteeringCommand {
        id: CommandId::from_bytes([4; 16]),
        workspace_id: 7,
        action: SteeringAction::Approve {
            approval_id: "approval-1".to_owned(),
        },
    };
    let mut ledger = CommandLedger::default();

    assert!(matches!(
        ledger.begin(&registry, client.id(), &command),
        Err(RemoteError::CapabilityDenied)
    ));
    assert!(registry.revoke(client.id()));
    assert!(matches!(
        registry.authorize(client.id(), 7, Capability::Prompt),
        Err(RemoteError::RevokedDevice)
    ));

    let session_id = SessionId::from_bytes([14; 16]);
    let mut host_session = RemoteSession::new(&client_on_host, session_id);
    let mut client_session = RemoteSession::new(&host_on_client, session_id);
    let packet = client_session
        .send_command(prompt_command(5, "revoked"))
        .unwrap();
    assert!(matches!(
        host_session.receive(&registry, &packet),
        Err(RemoteError::RevokedDevice)
    ));
}

#[test]
fn reconnect_retransmits_unacknowledged_payload_without_redelivery() {
    let (_, _, client_on_host, host_on_client) = paired_devices();
    let session_id = SessionId::from_bytes([8; 16]);
    let mut host_session = RemoteSession::new(&client_on_host, session_id);
    let mut client_session = RemoteSession::new(&host_on_client, session_id);
    let mut host_registry = DeviceRegistry::default();
    host_registry.pair(client_on_host);
    let mut client_registry = DeviceRegistry::default();
    client_registry.pair(host_on_client);
    let first = host_session.send_command(prompt_command(1, "one")).unwrap();

    assert!(matches!(
        client_session.receive(&client_registry, &first).unwrap(),
        InboundPacket::Delivery { sequence: 1, .. }
    ));
    assert_eq!(host_session.reconnect(), vec![first.clone()]);
    assert!(matches!(
        client_session.receive(&client_registry, &first).unwrap(),
        InboundPacket::Duplicate {
            acknowledged_sequence: 0
        }
    ));

    client_session.commit_received(1).unwrap();
    let ack = client_session.acknowledgement().unwrap();
    assert!(matches!(
        host_session.receive(&host_registry, &ack).unwrap(),
        InboundPacket::Acknowledgement {
            acknowledged_sequence: 0
        }
    ));
    assert_eq!(host_session.pending_outbound(), 0);
    assert!(host_session.reconnect().is_empty());

    let recovered = RemoteSession::from_checkpoint(client_session.checkpoint());
    assert_eq!(recovered.committed_inbound(), 1);
}

#[test]
fn hostile_acknowledgements_and_offline_queue_growth_are_bounded() {
    let (_, _, client_on_host, host_on_client) = paired_devices();
    let session_id = SessionId::from_bytes([13; 16]);
    let mut host_session = RemoteSession::new(&client_on_host, session_id);
    let mut client_session = RemoteSession::new(&host_on_client, session_id);
    let mut host_registry = DeviceRegistry::default();
    host_registry.pair(client_on_host);

    let mut ack = client_session.acknowledgement().unwrap();
    ack.header.acknowledged_sequence = 1;
    assert!(matches!(
        host_session.receive(&host_registry, &ack),
        Err(RemoteError::InvalidPacket(_))
    ));

    for id in 0..64 {
        host_session
            .send_command(prompt_command(id, "queued"))
            .unwrap();
    }
    assert!(matches!(
        host_session.send_command(prompt_command(65, "too many")),
        Err(RemoteError::OutboxFull)
    ));
}

#[test]
fn command_ids_make_retries_idempotent() {
    let (_, client, client_on_host, _) = paired_devices();
    let mut registry = DeviceRegistry::default();
    registry.pair(client_on_host);
    let command = prompt_command(7, "build");
    let mut ledger = CommandLedger::default();

    assert_eq!(
        ledger.begin(&registry, client.id(), &command).unwrap(),
        CommandDisposition::Execute
    );
    assert_eq!(
        ledger.begin(&registry, client.id(), &command).unwrap(),
        CommandDisposition::Pending
    );
    let receipt = ledger
        .complete(command.id, CommandOutcome::Applied)
        .unwrap();
    assert_eq!(
        ledger.begin(&registry, client.id(), &command).unwrap(),
        CommandDisposition::Completed(receipt)
    );
}

#[test]
fn steering_dispatches_once_and_replays_the_durable_receipt() {
    #[derive(Default)]
    struct Handler {
        prompts: usize,
    }

    impl SteeringHandler for Handler {
        fn prompt(&mut self, _: u64, _: u64, _: &str) -> Result<(), String> {
            self.prompts += 1;
            Ok(())
        }

        fn approve(&mut self, _: u64, _: &str) -> Result<(), String> {
            Ok(())
        }

        fn cancel(&mut self, _: u64, _: u64) -> Result<(), String> {
            Ok(())
        }

        fn resume(&mut self, _: u64, _: u64) -> Result<(), String> {
            Ok(())
        }
    }

    let directory = tempfile::tempdir().unwrap();
    let mut store = RemoteStateStore::open(directory.path().join(REMOTE_STATE_FILE_NAME)).unwrap();
    let mut state = store.load_or_create("Desktop").unwrap();
    let (_, client, client_on_host, _) = paired_devices();
    store.pair_device(&mut state, client_on_host).unwrap();
    let command = prompt_command(21, "run once");
    let mut handler = Handler::default();

    assert!(matches!(
        handle_command(&mut store, &mut state, client.id(), &command, &mut handler).unwrap(),
        CommandHandling::Executed(CommandReceipt {
            outcome: CommandOutcome::Applied,
            ..
        })
    ));
    assert!(matches!(
        handle_command(&mut store, &mut state, client.id(), &command, &mut handler).unwrap(),
        CommandHandling::Replayed(CommandReceipt {
            outcome: CommandOutcome::Applied,
            ..
        })
    ));
    assert_eq!(handler.prompts, 1);
}

#[test]
fn stream_deltas_are_filtered_before_encryption() {
    let (_, client, client_on_host, _) = paired_devices();
    let mut registry = DeviceRegistry::default();
    registry.pair(client_on_host.clone());
    let allowed = StreamDelta::Task(TaskDelta {
        event_id: EventId::from_bytes([1; 16]),
        workspace_id: 7,
        task_id: 4,
        title: "Ship".to_owned(),
        state: "running".to_owned(),
    });
    let denied = StreamDelta::Canvas(CanvasDelta {
        event_id: EventId::from_bytes([2; 16]),
        workspace_id: 7,
        revision: 1,
        selected_node_ids: vec![2],
        encoded_state: "{}".to_owned(),
    });

    assert!(registry.authorize_delta(client.id(), &allowed).is_ok());
    assert!(matches!(
        registry.authorize_delta(client.id(), &denied),
        Err(RemoteError::CapabilityDenied)
    ));

    let mut session = RemoteSession::new(&client_on_host, SessionId::from_bytes([12; 16]));
    assert!(matches!(
        session.send_event(&registry, denied),
        Err(RemoteError::CapabilityDenied)
    ));
}

#[test]
fn remote_identity_grants_sessions_and_receipts_survive_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(REMOTE_STATE_FILE_NAME);
    let mut store = RemoteStateStore::open(&path).unwrap();
    let mut state = store.load_or_create("Desktop").unwrap();
    let (client, client_on_host, _) = pair_with_host(state.identity());
    let identity_id = state.identity().id();
    store
        .pair_device(&mut state, client_on_host.clone())
        .unwrap();

    let command = SteeringCommand {
        id: CommandId::from_bytes([9; 16]),
        workspace_id: 7,
        action: SteeringAction::Cancel { task_id: 12 },
    };
    assert_eq!(
        store
            .begin_command(&mut state, client.id(), &command)
            .unwrap(),
        CommandDisposition::Execute
    );
    store
        .complete_command(&mut state, command.id, CommandOutcome::Applied)
        .unwrap();
    let mut session = store.start_session(&mut state, client.id()).unwrap();
    let session_id = session.checkpoint().session_id;
    store
        .encrypt_command(&mut state, &mut session, prompt_command(10, "persist me"))
        .unwrap();
    drop(store);

    let mut reopened = RemoteStateStore::open(&path).unwrap();
    let mut recovered = reopened.load_or_create("Ignored").unwrap();
    assert_eq!(recovered.identity().id(), identity_id);
    assert!(recovered.registry().device(client.id()).is_some());
    assert_eq!(
        reopened
            .begin_command(&mut recovered, client.id(), &command)
            .unwrap(),
        CommandDisposition::Completed(CommandReceipt {
            command_id: command.id,
            outcome: CommandOutcome::Applied,
        })
    );
    let restored_session =
        RemoteSession::from_checkpoint(recovered.checkpoint(session_id).unwrap().clone());
    assert_eq!(restored_session.pending_outbound(), 1);
    assert!(reopened.revoke_device(&mut recovered, client.id()).unwrap());
    drop(reopened);
    let mut after_revoke = RemoteStateStore::open(&path).unwrap();
    let revoked = after_revoke.load_or_create("Ignored").unwrap();
    assert!(matches!(
        revoked
            .registry()
            .authorize(client.id(), 7, Capability::Prompt),
        Err(RemoteError::RevokedDevice)
    ));
    assert!(revoked.checkpoint(session_id).is_none());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o077,
            0
        );
    }
}

#[cfg(unix)]
#[test]
fn remote_state_rejects_symlinks_and_group_readable_files() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let directory = tempfile::tempdir().unwrap();
    let state_path = directory.path().join("state.sqlite");
    let mut store = RemoteStateStore::open(&state_path).unwrap();
    store.load_or_create("Desktop").unwrap();
    drop(store);

    std::fs::set_permissions(&state_path, std::fs::Permissions::from_mode(0o640)).unwrap();
    assert!(matches!(
        RemoteStateStore::open(&state_path),
        Err(RemoteStorageError::InsecurePermissions(_))
    ));

    let link_path = directory.path().join("state-link.sqlite");
    symlink(&state_path, &link_path).unwrap();
    assert!(matches!(
        RemoteStateStore::open(&link_path),
        Err(RemoteStorageError::UnsafeFileType(_))
    ));
}

proptest! {
    #[test]
    fn arbitrary_remote_packets_are_rejected_or_structurally_bounded(bytes in prop::collection::vec(any::<u8>(), 0..20_000)) {
        if let Ok(packet) = EncryptedPacket::decode(&bytes) {
            prop_assert!(packet.ciphertext.len() <= MAX_PACKET_BYTES + 16);
            prop_assert_eq!(packet.header.version, PROTOCOL_VERSION);
            prop_assert!(packet.header.nonce_counter > 0);
        }
    }

    #[test]
    fn arbitrary_pairing_codes_fail_safely(code in ".{0,10000}") {
        let _ = PairingInvitation::from_code(&code);
    }
}
