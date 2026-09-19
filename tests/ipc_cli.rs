use std::process::Command;

use openpodium::ipc::{
    AGENT_ID_ENV, AVAILABLE_ENV, AgentCapabilities, AgentRegistration, ENDPOINT_ENV, IpcService,
    ProtocolResponse, ProtocolResult, TOKEN_ENV, VERSIONS_ENV, WORKSPACE_ID_ENV,
};
use tempfile::TempDir;

fn service() -> (TempDir, IpcService) {
    let temp = TempDir::new().unwrap();
    let service = IpcService::start(temp.path()).unwrap();
    service.replace_workspace_agents(
        4,
        [
            AgentRegistration {
                id: 7,
                name: "Builder".to_owned(),
                program: "Codex".to_owned(),
                state: "running".to_owned(),
                capabilities: AgentCapabilities::CONNECTED,
            },
            AgentRegistration {
                id: 8,
                name: "Reviewer".to_owned(),
                program: "Claude".to_owned(),
                state: "waiting".to_owned(),
                capabilities: AgentCapabilities::CONNECTED,
            },
        ],
    );
    (temp, service)
}

#[test]
fn binary_dispatches_ipc_commands_without_opening_the_desktop() {
    let (_temp, service) = service();
    let connection = service.connection_info(4, 7).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_openpodium"))
        .args(["ipc", "agents", "list"])
        .env(AVAILABLE_ENV, "1")
        .env(ENDPOINT_ENV, connection.endpoint().to_string())
        .env(TOKEN_ENV, connection.token())
        .env(VERSIONS_ENV, "1")
        .env(WORKSPACE_ID_ENV, "4")
        .env(AGENT_ID_ENV, "7")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let response: ProtocolResponse = serde_json::from_slice(&output.stdout).unwrap();
    assert!(matches!(
        response.result,
        Some(ProtocolResult::Agents { agents })
            if agents.len() == 2 && agents.iter().any(|agent| agent.id == 7 && agent.is_self)
    ));
}

#[test]
fn binary_uses_distinct_exit_status_for_rejected_authentication() {
    let (_temp, service) = service();
    let connection = service.connection_info(4, 7).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_openpodium"))
        .args(["ipc", "agents", "list"])
        .env(AVAILABLE_ENV, "1")
        .env(ENDPOINT_ENV, connection.endpoint().to_string())
        .env(TOKEN_ENV, "invalid")
        .env(VERSIONS_ENV, "1")
        .env(WORKSPACE_ID_ENV, "4")
        .env(AGENT_ID_ENV, "7")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(5));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unauthorized"));
    assert!(output.stdout.is_empty());
}
