use std::sync::Barrier;

use tempfile::TempDir;

use super::*;

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
    assert_eq!(error.supported_versions, Some(vec![PROTOCOL_VERSION]));
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
