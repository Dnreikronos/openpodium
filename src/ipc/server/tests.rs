use std::convert::Infallible;
use std::sync::{Arc, Barrier, Mutex};

use tempfile::TempDir;

use super::*;
use crate::ipc::{
    HandoffKind, LEGACY_PROTOCOL_VERSION, PortalActionRequest, PortalActionState,
    PortalPolicyOutcome, PortalScope, ResponseStatus,
};
use crate::portal::{
    PortalAction as CoreAction, PortalBackend, PortalCapabilities, PortalConfig,
    PortalObservation as CoreObservation, PortalSession,
};

fn registration(id: u64, name: &str) -> AgentRegistration {
    AgentRegistration {
        id,
        name: name.to_owned(),
        program: "Codex".to_owned(),
        state: "running".to_owned(),
        capabilities: AgentCapabilities::CONNECTED,
    }
}

fn request(
    service: &IpcService,
    workspace_id: u64,
    agent_id: u64,
    request_id: &str,
    command: ProtocolCommand,
) -> ProtocolRequest {
    ProtocolRequest::new(
        MessageId::new(request_id).unwrap(),
        service
            .connection_info(workspace_id, agent_id)
            .unwrap()
            .credentials(),
        command,
    )
}

fn round_trip(endpoint: SocketAddr, request: &ProtocolRequest) -> ProtocolResponse {
    let mut stream = TcpStream::connect(endpoint).unwrap();
    serde_json::to_writer(&mut stream, request).unwrap();
    stream.write_all(b"\n").unwrap();
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response).unwrap();
    serde_json::from_str(&response).unwrap()
}

fn service() -> (TempDir, IpcService) {
    let temp = TempDir::new().unwrap();
    let service = IpcService::start(temp.path()).unwrap();
    service.replace_workspace_agents(1, [registration(1, "Lead"), registration(2, "Builder")]);
    service.replace_workspace_agents(2, [registration(3, "Hidden")]);
    (temp, service)
}

#[derive(Clone, Default)]
struct RecordingPortalBackend {
    executed: Arc<Mutex<Vec<CoreAction>>>,
}

impl PortalBackend for RecordingPortalBackend {
    type Error = Infallible;

    fn connect(
        &mut self,
        _config: &PortalConfig,
        session: &mut PortalSession,
    ) -> Result<(), Self::Error> {
        session.begin_connect().unwrap();
        session.connected().unwrap();
        Ok(())
    }

    fn observe(&mut self, session: &mut PortalSession) -> Result<CoreObservation, Self::Error> {
        let revision = session.observe().unwrap();
        let observation = CoreObservation::new(revision, PortalCapabilities::browser_defaults());
        session.record_observation(observation.clone()).unwrap();
        Ok(observation)
    }

    fn execute(
        &mut self,
        _session: &mut PortalSession,
        action: &CoreAction,
    ) -> Result<(), Self::Error> {
        self.executed.lock().unwrap().push(action.clone());
        Ok(())
    }

    fn close(&mut self, session: &mut PortalSession) -> Result<(), Self::Error> {
        session.begin_close().unwrap();
        session.closed();
        Ok(())
    }
}

fn connect_test_portal(service: &IpcService) -> Arc<Mutex<Vec<CoreAction>>> {
    let executed = Arc::new(Mutex::new(Vec::new()));
    service
        .register_portal(
            10,
            PortalScope::new(1, None, 99).unwrap(),
            PortalConfig::browser("https://example.test").unwrap(),
            RecordingPortalBackend {
                executed: Arc::clone(&executed),
            },
        )
        .unwrap();
    service.attach_portal_agent(10, 1).unwrap();
    service.connect_portal(10).unwrap();
    executed
}

#[test]
fn agent_listing_is_scoped_to_the_authenticated_workspace() {
    let (_temp, service) = service();
    let response = round_trip(
        service.endpoint(),
        &request(&service, 1, 1, "list-1", ProtocolCommand::ListAgents),
    );

    let Some(ProtocolResult::Agents { agents }) = response.result else {
        panic!("expected an agent list: {response:?}");
    };
    assert_eq!(agents.len(), 2);
    assert!(agents.iter().all(|agent| agent.id != 3));
    assert!(agents.iter().any(|agent| agent.id == 1 && agent.is_self));
}

#[test]
fn portal_listing_is_scoped_to_the_authenticated_agent() {
    let (_temp, service) = service();
    let response = round_trip(
        service.endpoint(),
        &request(
            &service,
            1,
            1,
            "portal-list-1",
            ProtocolCommand::ListPortals,
        ),
    );

    assert_eq!(response.version, Some(PROTOCOL_VERSION));
    assert!(matches!(
        response.result,
        Some(ProtocolResult::Portals { portals }) if portals.is_empty()
    ));
    assert!(service.try_recv().is_none());
}

#[test]
fn connected_portals_reject_other_agents_and_workspaces() {
    let (_temp, service) = service();
    connect_test_portal(&service);

    let visible = round_trip(
        service.endpoint(),
        &request(
            &service,
            1,
            1,
            "portal-list-visible",
            ProtocolCommand::ListPortals,
        ),
    );
    assert!(matches!(
        visible.result,
        Some(ProtocolResult::Portals { portals }) if portals.len() == 1
    ));

    for (workspace_id, agent_id, request_id) in [
        (1, 2, "portal-inspect-unattached"),
        (2, 3, "portal-inspect-workspace"),
    ] {
        let response = round_trip(
            service.endpoint(),
            &request(
                &service,
                workspace_id,
                agent_id,
                request_id,
                ProtocolCommand::InspectPortal { portal_id: 10 },
            ),
        );
        assert_eq!(response.error.unwrap().code, ErrorCode::PortalUnavailable);
    }
}

#[test]
fn portal_action_flows_from_observation_through_approval_to_receipt() {
    let (_temp, service) = service();
    let executed = connect_test_portal(&service);

    let observed = round_trip(
        service.endpoint(),
        &request(
            &service,
            1,
            1,
            "portal-observe",
            ProtocolCommand::ObservePortal { portal_id: 10 },
        ),
    );
    let Some(ProtocolResult::PortalObservation(observation)) = observed.result else {
        panic!("expected a portal observation: {observed:?}");
    };

    let action_id = MessageId::new("portal-action-1").unwrap();
    let requested = round_trip(
        service.endpoint(),
        &request(
            &service,
            1,
            1,
            "portal-action-request",
            ProtocolCommand::RequestPortalAction {
                action_id: action_id.clone(),
                portal_id: 10,
                action: PortalActionRequest::Click {
                    element_id: "submit".to_owned(),
                    observation_revision: observation.revision,
                },
            },
        ),
    );
    let Some(ProtocolResult::PortalReceipt(receipt)) = requested.result else {
        panic!("expected an approval receipt: {requested:?}");
    };
    assert_eq!(receipt.state, PortalActionState::AwaitingApproval);
    let approval_id = match receipt.policy {
        PortalPolicyOutcome::ApprovalRequired { approval_id, .. } => approval_id,
        policy => panic!("unexpected portal policy: {policy:?}"),
    };

    let approved = service
        .approve_portal_action(1, &action_id, approval_id, 1_000)
        .unwrap();
    assert_eq!(approved.state, PortalActionState::Completed);
    assert_eq!(executed.lock().unwrap().len(), 1);

    let fetched = round_trip(
        service.endpoint(),
        &request(
            &service,
            1,
            1,
            "portal-result",
            ProtocolCommand::GetPortalResult { action_id },
        ),
    );
    assert!(matches!(
        fetched.result,
        Some(ProtocolResult::PortalReceipt(receipt))
            if receipt.state == PortalActionState::Completed
    ));
}

#[test]
fn invalid_credentials_are_rejected_without_identity_details() {
    let (_temp, service) = service();
    let mut unauthorized = request(&service, 1, 1, "list-1", ProtocolCommand::ListAgents);
    unauthorized.credentials.agent_id = 2;

    let response = round_trip(service.endpoint(), &unauthorized);

    let error = response.error.unwrap();
    assert_eq!(error.code, ErrorCode::Unauthorized);
    assert_eq!(error.message, "IPC authentication failed");
}

#[test]
fn incompatible_versions_return_supported_versions_and_upgrade_guidance() {
    let (_temp, service) = service();
    let mut incompatible = request(&service, 1, 1, "list-1", ProtocolCommand::ListAgents);
    incompatible.supported_versions = vec![99];

    let response = round_trip(service.endpoint(), &incompatible);

    let error = response.error.unwrap();
    assert_eq!(error.code, ErrorCode::IncompatibleProtocol);
    assert_eq!(error.supported_versions, Some(SUPPORTED_VERSIONS.to_vec()));
    assert!(error.message.contains("upgrade"));
}

#[test]
fn dropping_the_service_stops_accept_and_worker_threads_promptly() {
    let (_temp, service) = service();
    let started = std::time::Instant::now();

    drop(service);

    assert!(started.elapsed() < CONNECTION_TIMEOUT);
}

#[test]
fn identical_retries_publish_once_and_conflicting_reuse_is_rejected() {
    let (_temp, service) = service();
    let command = ProtocolCommand::SendTask {
        message_id: MessageId::new("task-1").unwrap(),
        recipient_agent_id: 2,
        title: "Build IPC".to_owned(),
        body: "Implement the local service".to_owned(),
    };
    let first = round_trip(
        service.endpoint(),
        &request(&service, 1, 1, "request-1", command.clone()),
    );
    let retry = round_trip(
        service.endpoint(),
        &request(&service, 1, 1, "request-2", command),
    );

    assert!(
        matches!(
            first.result,
            Some(ProtocolResult::Accepted {
                duplicate: false,
                ..
            })
        ),
        "unexpected first response: {first:?}"
    );
    assert!(matches!(
        retry.result,
        Some(ProtocolResult::Accepted {
            duplicate: true,
            ..
        })
    ));
    assert!(service.try_recv().is_some());
    assert!(service.try_recv().is_none());

    let conflict = ProtocolCommand::SendTask {
        message_id: MessageId::new("task-1").unwrap(),
        recipient_agent_id: 2,
        title: "Different".to_owned(),
        body: "Different payload".to_owned(),
    };
    let response = round_trip(
        service.endpoint(),
        &request(&service, 1, 1, "request-3", conflict),
    );
    assert_eq!(response.error.unwrap().code, ErrorCode::IdempotencyConflict);
}

#[test]
fn accepted_messages_and_idempotency_survive_service_restart() {
    let temp = TempDir::new().unwrap();
    let command = ProtocolCommand::SendTask {
        message_id: MessageId::new("task-durable").unwrap(),
        recipient_agent_id: 2,
        title: "Persist me".to_owned(),
        body: "Remain pending across restart".to_owned(),
    };

    {
        let service = IpcService::start(temp.path()).unwrap();
        service.replace_workspace_agents(1, [registration(1, "Lead"), registration(2, "Builder")]);
        let response = round_trip(
            service.endpoint(),
            &request(&service, 1, 1, "request-1", command.clone()),
        );
        assert!(matches!(
            response.result,
            Some(ProtocolResult::Accepted {
                duplicate: false,
                ..
            })
        ));
    }

    {
        let service = IpcService::start(temp.path()).unwrap();
        service.replace_workspace_agents(1, [registration(1, "Lead"), registration(2, "Builder")]);
        let retry = round_trip(
            service.endpoint(),
            &request(&service, 1, 1, "request-2", command.clone()),
        );
        assert!(matches!(
            retry.result,
            Some(ProtocolResult::Accepted {
                duplicate: true,
                ..
            })
        ));

        let recovered = service.next_message().unwrap().unwrap();
        assert_eq!(recovered.command, command);
        assert!(service.mark_processed(&recovered).unwrap());
    }

    let service = IpcService::start(temp.path()).unwrap();
    service.replace_workspace_agents(1, [registration(1, "Lead"), registration(2, "Builder")]);
    assert!(service.next_message().unwrap().is_none());
    let retry = round_trip(
        service.endpoint(),
        &request(&service, 1, 1, "request-3", command),
    );
    assert!(matches!(
        retry.result,
        Some(ProtocolResult::Accepted {
            duplicate: true,
            ..
        })
    ));
}

#[test]
fn progress_and_response_route_back_to_the_task_sender() {
    let (_temp, service) = service();
    let task = ProtocolCommand::SendTask {
        message_id: MessageId::new("task-1").unwrap(),
        recipient_agent_id: 2,
        title: "Build IPC".to_owned(),
        body: "Implement the local service".to_owned(),
    };
    round_trip(
        service.endpoint(),
        &request(&service, 1, 1, "request-1", task),
    );
    let progress = ProtocolCommand::ReportProgress {
        message_id: MessageId::new("progress-1").unwrap(),
        task_message_id: MessageId::new("task-1").unwrap(),
        body: "Halfway done".to_owned(),
    };
    let response = round_trip(
        service.endpoint(),
        &request(&service, 1, 2, "request-2", progress),
    );
    assert!(response.error.is_none());

    let task = service.try_recv().unwrap();
    let progress = service.try_recv().unwrap();
    assert_eq!(task.recipient_agent_id, 2);
    assert_eq!(progress.sender_agent_id, 2);
    assert_eq!(progress.recipient_agent_id, 1);
}

#[test]
fn version_two_questions_progress_responses_and_cancellation_are_routed() {
    let (_temp, service) = service();
    let question = ProtocolCommand::SendHandoff {
        message_id: MessageId::new("question-1").unwrap(),
        recipient_agent_id: 2,
        kind: HandoffKind::Question,
        title: None,
        body: "Which API should I use?".to_owned(),
        parent_message_id: None,
        response_timeout_ms: Some(30_000),
    };
    let sent = round_trip(
        service.endpoint(),
        &request(&service, 1, 1, "request-1", question),
    );
    assert_eq!(sent.version, Some(PROTOCOL_VERSION));

    for (request_id, command) in [
        (
            "request-2",
            ProtocolCommand::ReportHandoffProgress {
                message_id: MessageId::new("progress-1").unwrap(),
                handoff_message_id: MessageId::new("question-1").unwrap(),
                body: "Checking the domain model".to_owned(),
            },
        ),
        (
            "request-3",
            ProtocolCommand::RespondToHandoff {
                message_id: MessageId::new("response-1").unwrap(),
                handoff_message_id: MessageId::new("question-1").unwrap(),
                status: ResponseStatus::Completed,
                body: "Use the workspace aggregate".to_owned(),
            },
        ),
    ] {
        let response = round_trip(
            service.endpoint(),
            &request(&service, 1, 2, request_id, command),
        );
        assert!(
            response.error.is_none(),
            "unexpected response: {response:?}"
        );
    }

    let cancellation = round_trip(
        service.endpoint(),
        &request(
            &service,
            1,
            1,
            "request-4",
            ProtocolCommand::CancelHandoff {
                message_id: MessageId::new("cancel-1").unwrap(),
                handoff_message_id: MessageId::new("question-1").unwrap(),
                reason: "Answered out of band".to_owned(),
            },
        ),
    );
    assert!(cancellation.error.is_none());

    let messages: Vec<_> = std::iter::from_fn(|| service.try_recv()).collect();
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0].recipient_agent_id, 2);
    assert_eq!(messages[1].recipient_agent_id, 1);
    assert_eq!(messages[2].recipient_agent_id, 1);
    assert_eq!(messages[3].recipient_agent_id, 2);
}

#[test]
fn legacy_clients_negotiate_version_one_and_cannot_send_v2_commands() {
    let (_temp, service) = service();
    let mut list = request(&service, 1, 1, "request-1", ProtocolCommand::ListAgents);
    list.supported_versions = vec![LEGACY_PROTOCOL_VERSION];
    let response = round_trip(service.endpoint(), &list);
    assert_eq!(response.version, Some(LEGACY_PROTOCOL_VERSION));

    let mut question = request(
        &service,
        1,
        1,
        "request-2",
        ProtocolCommand::SendHandoff {
            message_id: MessageId::new("question-1").unwrap(),
            recipient_agent_id: 2,
            kind: HandoffKind::Question,
            title: None,
            body: "Question".to_owned(),
            parent_message_id: None,
            response_timeout_ms: None,
        },
    );
    question.supported_versions = vec![LEGACY_PROTOCOL_VERSION];
    let response = round_trip(service.endpoint(), &question);
    assert_eq!(
        response.error.unwrap().code,
        ErrorCode::IncompatibleProtocol
    );
}

#[test]
fn unavailable_adapters_reject_handoffs_before_they_enter_the_queue() {
    let (_temp, service) = service();
    service.replace_workspace_agents(
        1,
        [
            registration(1, "Lead"),
            AgentRegistration {
                capabilities: AgentCapabilities::UNAVAILABLE,
                ..registration(2, "Shell")
            },
        ],
    );

    let response = round_trip(
        service.endpoint(),
        &request(
            &service,
            1,
            1,
            "request-1",
            ProtocolCommand::SendHandoff {
                message_id: MessageId::new("question-1").unwrap(),
                recipient_agent_id: 2,
                kind: HandoffKind::Question,
                title: None,
                body: "Can you receive this?".to_owned(),
                parent_message_id: None,
                response_timeout_ms: None,
            },
        ),
    );

    assert_eq!(response.error.unwrap().code, ErrorCode::InvalidRequest);
    assert!(service.next_message().unwrap().is_none());
}

#[test]
fn concurrent_clients_publish_distinct_messages() {
    let (_temp, service) = service();
    let client_count = 16;
    let barrier = Arc::new(Barrier::new(client_count));
    let endpoint = service.endpoint();
    let credentials = service.connection_info(1, 1).unwrap().credentials();
    let handles: Vec<_> = (0..client_count)
        .map(|index| {
            let barrier = Arc::clone(&barrier);
            let credentials = credentials.clone();
            thread::spawn(move || {
                let request = ProtocolRequest::new(
                    MessageId::new(format!("request-{index}")).unwrap(),
                    credentials,
                    ProtocolCommand::SendTask {
                        message_id: MessageId::new(format!("task-{index}")).unwrap(),
                        recipient_agent_id: 2,
                        title: format!("Task {index}"),
                        body: "Concurrent work".to_owned(),
                    },
                );
                barrier.wait();
                round_trip(endpoint, &request)
            })
        })
        .collect();

    for handle in handles {
        assert!(handle.join().unwrap().error.is_none());
    }
    assert_eq!(
        std::iter::from_fn(|| service.try_recv()).count(),
        client_count
    );
}

#[test]
fn malformed_and_oversized_frames_return_stable_errors() {
    let (_temp, service) = service();
    let mut malformed = TcpStream::connect(service.endpoint()).unwrap();
    malformed.write_all(b"not json\n").unwrap();
    let mut response = String::new();
    BufReader::new(malformed).read_line(&mut response).unwrap();
    let response: ProtocolResponse = serde_json::from_str(&response).unwrap();
    assert_eq!(response.error.unwrap().code, ErrorCode::MalformedRequest);

    let mut oversized = TcpStream::connect(service.endpoint()).unwrap();
    oversized
        .write_all(&vec![b'x'; MAX_FRAME_BYTES + 1])
        .unwrap();
    oversized.write_all(b"\n").unwrap();
    let mut response = String::new();
    BufReader::new(oversized).read_line(&mut response).unwrap();
    let response: ProtocolResponse = serde_json::from_str(&response).unwrap();
    assert_eq!(response.error.unwrap().code, ErrorCode::FrameTooLarge);
}
