use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use std::io::{self, Write};

use super::{
    ClientError, ConnectionConfig, ErrorCode, HandoffKind, IpcClient, MessageId,
    PortalActionRequest, ProtocolCommand, ProtocolResponse, ResponseStatus,
};

const EXIT_SUCCESS: u8 = 0;
const EXIT_USAGE: u8 = 2;
const EXIT_ENVIRONMENT: u8 = 3;
const EXIT_CONNECTION: u8 = 4;
const EXIT_AUTHENTICATION: u8 = 5;
const EXIT_COMPATIBILITY: u8 = 6;
const EXIT_REJECTED: u8 = 7;

const USAGE: &str = "Usage:
  openpodium ipc agents list
  openpodium ipc portals list
  openpodium ipc portal inspect --portal <portal-id>
  openpodium ipc portal observe --portal <portal-id>
  openpodium ipc portal click --portal <portal-id> --element <element-id> --revision <revision> [--action-id <id>]
  openpodium ipc portal click-coordinate --portal <portal-id> --x <pixels> --y <pixels> --revision <revision> [--action-id <id>]
  openpodium ipc portal type --portal <portal-id> --element <element-id> --revision <revision> --text <text> [--action-id <id>]
  openpodium ipc portal scroll --portal <portal-id> --revision <revision> --delta-x <pixels> --delta-y <pixels> [--element <element-id>] [--action-id <id>]
  openpodium ipc portal scroll-coordinate --portal <portal-id> --x <pixels> --y <pixels> --revision <revision> --delta-x <pixels> --delta-y <pixels> [--action-id <id>]
  openpodium ipc portal navigate --portal <portal-id> --target <url> [--action-id <id>]
  openpodium ipc portal result --action <action-id>
  openpodium ipc task send --to <agent-id> --title <title> --body <text> [--parent <handoff-id>] [--timeout-ms <milliseconds>] [--message-id <id>]
  openpodium ipc question send --to <agent-id> --body <text> [--parent <handoff-id>] [--timeout-ms <milliseconds>] [--message-id <id>]
  openpodium ipc progress report --handoff <message-id> --body <text> [--message-id <id>]
  openpodium ipc respond --handoff <message-id> --status <completed|failed|blocked> --body <text> [--message-id <id>]
  openpodium ipc cancel --handoff <message-id> --reason <text> [--message-id <id>]";

pub fn run_cli(arguments: impl IntoIterator<Item = String>) -> u8 {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    run(arguments, &mut stdout, &mut stderr)
}

fn run(
    arguments: impl IntoIterator<Item = String>,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> u8 {
    let arguments: Vec<String> = arguments.into_iter().collect();
    let command = match parse_command(&arguments) {
        Ok(command) => command,
        Err(error) => {
            let _ = writeln!(stderr, "{error}\n\n{USAGE}");
            return EXIT_USAGE;
        }
    };
    let config = match ConnectionConfig::from_environment() {
        Ok(config) => config,
        Err(error) => {
            let _ = writeln!(stderr, "{error}");
            return if error.is_environment_error() {
                EXIT_ENVIRONMENT
            } else {
                EXIT_CONNECTION
            };
        }
    };
    let request_id = match random_id("request") {
        Ok(request_id) => request_id,
        Err(error) => {
            let _ = writeln!(stderr, "{error}");
            return EXIT_CONNECTION;
        }
    };
    let response = match IpcClient::new(config).send(request_id, command) {
        Ok(response) => response,
        Err(error) => {
            let _ = writeln!(stderr, "{error}");
            return if error.is_environment_error() {
                EXIT_ENVIRONMENT
            } else {
                EXIT_CONNECTION
            };
        }
    };
    write_response(response, stdout, stderr)
}

fn write_response(
    response: ProtocolResponse,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> u8 {
    if let Some(error) = &response.error {
        let _ = writeln!(stderr, "{}: {}", error_code(error.code), error.message);
        return match error.code {
            ErrorCode::Unauthorized => EXIT_AUTHENTICATION,
            ErrorCode::IncompatibleProtocol => EXIT_COMPATIBILITY,
            _ => EXIT_REJECTED,
        };
    }
    match serde_json::to_writer(&mut *stdout, &response) {
        Ok(()) => {
            let _ = writeln!(stdout);
            EXIT_SUCCESS
        }
        Err(error) => {
            let _ = writeln!(stderr, "failed to encode CLI response: {error}");
            EXIT_CONNECTION
        }
    }
}

fn parse_command(arguments: &[String]) -> Result<ProtocolCommand, CliUsageError> {
    let words: Vec<&str> = arguments.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["ipc", "agents", "list"] => Ok(ProtocolCommand::ListAgents),
        ["ipc", "portals", "list"] => Ok(ProtocolCommand::ListPortals),
        ["ipc", "portal", "inspect", options @ ..] => {
            let options = parse_options(options, &["--portal"])?;
            Ok(ProtocolCommand::InspectPortal {
                portal_id: portal_id(&options)?,
            })
        }
        ["ipc", "portal", "observe", options @ ..] => {
            let options = parse_options(options, &["--portal"])?;
            Ok(ProtocolCommand::ObservePortal {
                portal_id: portal_id(&options)?,
            })
        }
        ["ipc", "portal", "click", options @ ..] => {
            let options = parse_options(
                options,
                &["--portal", "--element", "--revision", "--action-id"],
            )?;
            Ok(ProtocolCommand::RequestPortalAction {
                action_id: action_id(&options)?,
                portal_id: portal_id(&options)?,
                action: PortalActionRequest::Click {
                    element_id: required(&options, "--element")?.to_owned(),
                    observation_revision: positive_u64(&options, "--revision", "revision")?,
                },
            })
        }
        ["ipc", "portal", "click-coordinate", options @ ..] => {
            let options = parse_options(
                options,
                &["--portal", "--x", "--y", "--revision", "--action-id"],
            )?;
            Ok(ProtocolCommand::RequestPortalAction {
                action_id: action_id(&options)?,
                portal_id: portal_id(&options)?,
                action: PortalActionRequest::ClickCoordinate {
                    observation_revision: positive_u64(&options, "--revision", "revision")?,
                    x: unsigned_u32(&options, "--x")?,
                    y: unsigned_u32(&options, "--y")?,
                },
            })
        }
        ["ipc", "portal", "type", options @ ..] => {
            let options = parse_options(
                options,
                &[
                    "--portal",
                    "--element",
                    "--revision",
                    "--text",
                    "--action-id",
                ],
            )?;
            Ok(ProtocolCommand::RequestPortalAction {
                action_id: action_id(&options)?,
                portal_id: portal_id(&options)?,
                action: PortalActionRequest::TypeText {
                    element_id: required(&options, "--element")?.to_owned(),
                    observation_revision: positive_u64(&options, "--revision", "revision")?,
                    text: required(&options, "--text")?.to_owned(),
                },
            })
        }
        ["ipc", "portal", "scroll", options @ ..] => {
            let options = parse_options(
                options,
                &[
                    "--portal",
                    "--element",
                    "--revision",
                    "--delta-x",
                    "--delta-y",
                    "--action-id",
                ],
            )?;
            Ok(ProtocolCommand::RequestPortalAction {
                action_id: action_id(&options)?,
                portal_id: portal_id(&options)?,
                action: PortalActionRequest::Scroll {
                    element_id: options.get("--element").map(|value| (*value).to_owned()),
                    observation_revision: positive_u64(&options, "--revision", "revision")?,
                    delta_x: signed_i32(&options, "--delta-x")?,
                    delta_y: signed_i32(&options, "--delta-y")?,
                },
            })
        }
        ["ipc", "portal", "scroll-coordinate", options @ ..] => {
            let options = parse_options(
                options,
                &[
                    "--portal",
                    "--x",
                    "--y",
                    "--revision",
                    "--delta-x",
                    "--delta-y",
                    "--action-id",
                ],
            )?;
            Ok(ProtocolCommand::RequestPortalAction {
                action_id: action_id(&options)?,
                portal_id: portal_id(&options)?,
                action: PortalActionRequest::ScrollCoordinate {
                    observation_revision: positive_u64(&options, "--revision", "revision")?,
                    x: unsigned_u32(&options, "--x")?,
                    y: unsigned_u32(&options, "--y")?,
                    delta_x: signed_i32(&options, "--delta-x")?,
                    delta_y: signed_i32(&options, "--delta-y")?,
                },
            })
        }
        ["ipc", "portal", "navigate", options @ ..] => {
            let options = parse_options(options, &["--portal", "--target", "--action-id"])?;
            Ok(ProtocolCommand::RequestPortalAction {
                action_id: action_id(&options)?,
                portal_id: portal_id(&options)?,
                action: PortalActionRequest::Navigate {
                    target: required(&options, "--target")?.to_owned(),
                },
            })
        }
        ["ipc", "portal", "result", options @ ..] => {
            let options = parse_options(options, &["--action"])?;
            Ok(ProtocolCommand::GetPortalResult {
                action_id: required_id(&options, "--action")?,
            })
        }
        ["ipc", "task", "send", options @ ..] => {
            let options = parse_options(
                options,
                &[
                    "--to",
                    "--title",
                    "--body",
                    "--parent",
                    "--timeout-ms",
                    "--message-id",
                ],
            )?;
            Ok(ProtocolCommand::SendHandoff {
                message_id: message_id(&options, "task")?,
                recipient_agent_id: recipient_id(&options)?,
                kind: HandoffKind::Task,
                title: Some(required(&options, "--title")?.to_owned()),
                body: required(&options, "--body")?.to_owned(),
                parent_message_id: optional_id(&options, "--parent")?,
                response_timeout_ms: optional_timeout(&options)?,
            })
        }
        ["ipc", "question", "send", options @ ..] => {
            let options = parse_options(
                options,
                &["--to", "--body", "--parent", "--timeout-ms", "--message-id"],
            )?;
            Ok(ProtocolCommand::SendHandoff {
                message_id: message_id(&options, "question")?,
                recipient_agent_id: recipient_id(&options)?,
                kind: HandoffKind::Question,
                title: None,
                body: required(&options, "--body")?.to_owned(),
                parent_message_id: optional_id(&options, "--parent")?,
                response_timeout_ms: optional_timeout(&options)?,
            })
        }
        ["ipc", "progress", "report", options @ ..] => {
            let options = parse_options(options, &["--handoff", "--body", "--message-id"])?;
            Ok(ProtocolCommand::ReportHandoffProgress {
                message_id: message_id(&options, "progress")?,
                handoff_message_id: required_id(&options, "--handoff")?,
                body: required(&options, "--body")?.to_owned(),
            })
        }
        ["ipc", "respond", options @ ..] => {
            let options = parse_options(
                options,
                &["--handoff", "--status", "--body", "--message-id"],
            )?;
            let status = match required(&options, "--status")? {
                "completed" => ResponseStatus::Completed,
                "failed" => ResponseStatus::Failed,
                "blocked" => ResponseStatus::Blocked,
                _ => {
                    return Err(CliUsageError(
                        "--status must be completed, failed, or blocked".to_owned(),
                    ));
                }
            };
            Ok(ProtocolCommand::RespondToHandoff {
                message_id: message_id(&options, "response")?,
                handoff_message_id: required_id(&options, "--handoff")?,
                status,
                body: required(&options, "--body")?.to_owned(),
            })
        }
        ["ipc", "cancel", options @ ..] => {
            let options = parse_options(options, &["--handoff", "--reason", "--message-id"])?;
            Ok(ProtocolCommand::CancelHandoff {
                message_id: message_id(&options, "cancel")?,
                handoff_message_id: required_id(&options, "--handoff")?,
                reason: required(&options, "--reason")?.to_owned(),
            })
        }
        _ => Err(CliUsageError("unknown IPC command".to_owned())),
    }
}

fn parse_options<'a>(
    arguments: &'a [&'a str],
    allowed: &[&str],
) -> Result<BTreeMap<&'a str, &'a str>, CliUsageError> {
    let mut parsed = BTreeMap::new();
    let mut remaining = arguments;
    while let [name, value, tail @ ..] = remaining {
        if !allowed.contains(name) {
            return Err(CliUsageError(format!("unknown option {name}")));
        }
        if value.starts_with("--") {
            return Err(CliUsageError(format!("{name} requires a value")));
        }
        if parsed.insert(*name, *value).is_some() {
            return Err(CliUsageError(format!("{name} may be supplied only once")));
        }
        remaining = tail;
    }
    if !remaining.is_empty() {
        return Err(CliUsageError(format!("{} requires a value", remaining[0])));
    }
    Ok(parsed)
}

fn required<'a>(
    options: &'a BTreeMap<&str, &'a str>,
    name: &str,
) -> Result<&'a str, CliUsageError> {
    options
        .get(name)
        .copied()
        .ok_or_else(|| CliUsageError(format!("missing required option {name}")))
}

fn message_id(options: &BTreeMap<&str, &str>, prefix: &str) -> Result<MessageId, CliUsageError> {
    options.get("--message-id").map_or_else(
        || random_id(prefix).map_err(|error| CliUsageError(error.to_string())),
        |value| {
            MessageId::new((*value).to_owned())
                .map_err(|error| CliUsageError(format!("invalid --message-id: {error}")))
        },
    )
}

fn action_id(options: &BTreeMap<&str, &str>) -> Result<MessageId, CliUsageError> {
    options.get("--action-id").map_or_else(
        || random_id("portal-action").map_err(|error| CliUsageError(error.to_string())),
        |value| {
            MessageId::new((*value).to_owned())
                .map_err(|error| CliUsageError(format!("invalid --action-id: {error}")))
        },
    )
}

fn portal_id(options: &BTreeMap<&str, &str>) -> Result<u64, CliUsageError> {
    positive_u64(options, "--portal", "portal ID")
}

fn positive_u64(
    options: &BTreeMap<&str, &str>,
    name: &str,
    label: &str,
) -> Result<u64, CliUsageError> {
    required(options, name)?
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| CliUsageError(format!("{name} must be a positive {label}")))
}

fn signed_i32(options: &BTreeMap<&str, &str>, name: &str) -> Result<i32, CliUsageError> {
    required(options, name)?
        .parse::<i32>()
        .map_err(|_| CliUsageError(format!("{name} must be a 32-bit integer")))
}

fn unsigned_u32(options: &BTreeMap<&str, &str>, name: &str) -> Result<u32, CliUsageError> {
    required(options, name)?
        .parse::<u32>()
        .map_err(|_| CliUsageError(format!("{name} must be a non-negative 32-bit integer")))
}

fn recipient_id(options: &BTreeMap<&str, &str>) -> Result<u64, CliUsageError> {
    required(options, "--to")?
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| CliUsageError("--to must be a positive agent ID".to_owned()))
}

fn required_id(options: &BTreeMap<&str, &str>, name: &str) -> Result<MessageId, CliUsageError> {
    MessageId::new(required(options, name)?.to_owned())
        .map_err(|error| CliUsageError(format!("invalid {name}: {error}")))
}

fn optional_id(
    options: &BTreeMap<&str, &str>,
    name: &str,
) -> Result<Option<MessageId>, CliUsageError> {
    options
        .get(name)
        .map(|value| {
            MessageId::new((*value).to_owned())
                .map_err(|error| CliUsageError(format!("invalid {name}: {error}")))
        })
        .transpose()
}

fn optional_timeout(options: &BTreeMap<&str, &str>) -> Result<Option<u64>, CliUsageError> {
    options
        .get("--timeout-ms")
        .map(|value| {
            value
                .parse::<u64>()
                .ok()
                .filter(|timeout| *timeout > 0)
                .ok_or_else(|| CliUsageError("--timeout-ms must be a positive integer".to_owned()))
        })
        .transpose()
}

fn random_id(prefix: &str) -> Result<MessageId, ClientError> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|error| {
        ClientError::InvalidResponse(format!("failed to generate message ID: {error}"))
    })?;
    let suffix = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    MessageId::new(format!("{prefix}-{suffix}"))
        .map_err(|error| ClientError::InvalidResponse(error.to_string()))
}

const fn error_code(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::MalformedRequest => "malformed_request",
        ErrorCode::FrameTooLarge => "frame_too_large",
        ErrorCode::IncompatibleProtocol => "incompatible_protocol",
        ErrorCode::Unauthorized => "unauthorized",
        ErrorCode::InvalidRequest => "invalid_request",
        ErrorCode::AgentNotVisible => "agent_not_visible",
        ErrorCode::IdempotencyConflict => "idempotency_conflict",
        ErrorCode::ServiceUnavailable => "service_unavailable",
        ErrorCode::PortalUnavailable => "portal_unavailable",
        ErrorCode::PortalPolicyDenied => "portal_policy_denied",
        ErrorCode::PortalApprovalRequired => "portal_approval_required",
        ErrorCode::StalePortalObservation => "stale_portal_observation",
        ErrorCode::UnknownPortalAction => "unknown_portal_action",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliUsageError(String);

impl Display for CliUsageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn parses_each_documented_command_without_shell_interpolation() {
        assert_eq!(
            parse_command(&arguments(&["ipc", "agents", "list"])).unwrap(),
            ProtocolCommand::ListAgents
        );
        assert_eq!(
            parse_command(&arguments(&[
                "ipc",
                "task",
                "send",
                "--to",
                "8",
                "--title",
                "Review",
                "--body",
                "$(touch /tmp/unsafe)",
                "--message-id",
                "task-1",
            ]))
            .unwrap(),
            ProtocolCommand::SendHandoff {
                message_id: MessageId::new("task-1").unwrap(),
                recipient_agent_id: 8,
                kind: HandoffKind::Task,
                title: Some("Review".to_owned()),
                body: "$(touch /tmp/unsafe)".to_owned(),
                parent_message_id: None,
                response_timeout_ms: None,
            }
        );
        assert!(matches!(
            parse_command(&arguments(&[
                "ipc",
                "portal",
                "click-coordinate",
                "--portal",
                "9",
                "--x",
                "320",
                "--y",
                "240",
                "--revision",
                "4",
            ]))
            .unwrap(),
            ProtocolCommand::RequestPortalAction {
                action: PortalActionRequest::ClickCoordinate {
                    observation_revision: 4,
                    x: 320,
                    y: 240,
                },
                ..
            }
        ));
        assert!(matches!(
            parse_command(&arguments(&[
                "ipc",
                "question",
                "send",
                "--to",
                "7",
                "--body",
                "Which API?",
                "--parent",
                "task-1",
                "--timeout-ms",
                "30000",
            ]))
            .unwrap(),
            ProtocolCommand::SendHandoff {
                kind: HandoffKind::Question,
                parent_message_id: Some(_),
                response_timeout_ms: Some(30_000),
                ..
            }
        ));
        assert!(matches!(
            parse_command(&arguments(&[
                "ipc",
                "progress",
                "report",
                "--handoff",
                "task-1",
                "--body",
                "Running tests",
                "--message-id",
                "progress-1",
            ]))
            .unwrap(),
            ProtocolCommand::ReportHandoffProgress { .. }
        ));
        assert!(matches!(
            parse_command(&arguments(&[
                "ipc",
                "respond",
                "--handoff",
                "task-1",
                "--status",
                "completed",
                "--body",
                "Done",
                "--message-id",
                "response-1",
            ]))
            .unwrap(),
            ProtocolCommand::RespondToHandoff {
                status: ResponseStatus::Completed,
                ..
            }
        ));
        assert!(matches!(
            parse_command(&arguments(&[
                "ipc",
                "cancel",
                "--handoff",
                "task-1",
                "--reason",
                "No longer needed",
            ]))
            .unwrap(),
            ProtocolCommand::CancelHandoff { .. }
        ));
    }

    #[test]
    fn parses_portal_commands_with_semantic_references() {
        assert_eq!(
            parse_command(&arguments(&["ipc", "portals", "list"])).unwrap(),
            ProtocolCommand::ListPortals
        );
        assert_eq!(
            parse_command(&arguments(&["ipc", "portal", "inspect", "--portal", "9"])).unwrap(),
            ProtocolCommand::InspectPortal { portal_id: 9 }
        );
        assert_eq!(
            parse_command(&arguments(&["ipc", "portal", "observe", "--portal", "9"])).unwrap(),
            ProtocolCommand::ObservePortal { portal_id: 9 }
        );
        assert_eq!(
            parse_command(&arguments(&[
                "ipc",
                "portal",
                "click",
                "--portal",
                "9",
                "--element",
                "submit",
                "--revision",
                "4",
                "--action-id",
                "action-1",
            ]))
            .unwrap(),
            ProtocolCommand::RequestPortalAction {
                action_id: MessageId::new("action-1").unwrap(),
                portal_id: 9,
                action: PortalActionRequest::Click {
                    element_id: "submit".to_owned(),
                    observation_revision: 4,
                },
            }
        );
        assert!(matches!(
            parse_command(&arguments(&[
                "ipc",
                "portal",
                "scroll-coordinate",
                "--portal",
                "9",
                "--x",
                "320",
                "--y",
                "240",
                "--revision",
                "4",
                "--delta-x",
                "0",
                "--delta-y",
                "-120",
            ]))
            .unwrap(),
            ProtocolCommand::RequestPortalAction {
                action: PortalActionRequest::ScrollCoordinate { x: 320, y: 240, .. },
                ..
            }
        ));
        assert!(matches!(
            parse_command(&arguments(&[
                "ipc",
                "portal",
                "type",
                "--portal",
                "9",
                "--element",
                "search",
                "--revision",
                "4",
                "--text",
                "$(touch /tmp/unsafe)",
                "--action-id",
                "action-2",
            ]))
            .unwrap(),
            ProtocolCommand::RequestPortalAction {
                action: PortalActionRequest::TypeText { text, .. },
                ..
            } if text == "$(touch /tmp/unsafe)"
        ));
        assert_eq!(
            parse_command(&arguments(&[
                "ipc",
                "portal",
                "scroll",
                "--portal",
                "9",
                "--revision",
                "4",
                "--delta-x",
                "0",
                "--delta-y",
                "-120",
                "--action-id",
                "action-3",
            ]))
            .unwrap(),
            ProtocolCommand::RequestPortalAction {
                action_id: MessageId::new("action-3").unwrap(),
                portal_id: 9,
                action: PortalActionRequest::Scroll {
                    element_id: None,
                    observation_revision: 4,
                    delta_x: 0,
                    delta_y: -120,
                },
            }
        );
        assert!(matches!(
            parse_command(&arguments(&[
                "ipc",
                "portal",
                "navigate",
                "--portal",
                "9",
                "--target",
                "https://example.test",
                "--action-id",
                "action-4",
            ]))
            .unwrap(),
            ProtocolCommand::RequestPortalAction {
                action: PortalActionRequest::Navigate { .. },
                ..
            }
        ));
        assert_eq!(
            parse_command(&arguments(&[
                "ipc", "portal", "result", "--action", "action-4",
            ]))
            .unwrap(),
            ProtocolCommand::GetPortalResult {
                action_id: MessageId::new("action-4").unwrap(),
            }
        );
    }

    #[test]
    fn rejects_missing_duplicate_and_unknown_options() {
        let missing = parse_command(&arguments(&[
            "ipc", "task", "send", "--to", "8", "--title", "Review",
        ]));
        assert_eq!(
            missing.unwrap_err(),
            CliUsageError("missing required option --body".to_owned())
        );

        let duplicate = parse_command(&arguments(&[
            "ipc",
            "progress",
            "report",
            "--handoff",
            "task-1",
            "--body",
            "one",
            "--body",
            "two",
        ]));
        assert_eq!(
            duplicate.unwrap_err(),
            CliUsageError("--body may be supplied only once".to_owned())
        );

        let unknown = parse_command(&arguments(&["ipc", "agents", "remove"]));
        assert_eq!(
            unknown.unwrap_err(),
            CliUsageError("unknown IPC command".to_owned())
        );

        let invalid_portal =
            parse_command(&arguments(&["ipc", "portal", "observe", "--portal", "0"]));
        assert_eq!(
            invalid_portal.unwrap_err(),
            CliUsageError("--portal must be a positive portal ID".to_owned())
        );
    }
}
