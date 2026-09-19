# Authenticated local IPC and command-line client

Status: Issue #12 base contract, extended by issue #13
Updated: 2026-09-19

## Boundary

The IPC module owns a versioned local transport, authentication, workspace-scoped
agent discovery, structured message envelopes, retry deduplication, and the
command-line client used by launched agents. It publishes accepted messages to
an application-facing queue but does not create domain tasks, transition task
state, inject prompts into terminals, or persist delivery attempts. Typed
handoff orchestration and restart-safe delivery are owned by issue #13.

The desktop application starts one service for its process lifetime. The
service listens only on an operating-system-selected IPv4 loopback port. JSON
Lines provides framing: one request and one response per connection, each
terminated by a newline and limited to 1 MiB. Connections have bounded read and
write timeouts so malformed or abandoned clients cannot retain a worker
indefinitely.

## Authentication and visibility

OpenPodium creates a random 256-bit installation secret in its application-data
directory. Creation is exclusive; on Unix the file is created with mode `0600`,
and on Windows it inherits the user's application-data access controls. The
secret is never included in protocol responses, process previews, or diagnostic
errors.

Before launching a local agent, the application derives a capability token
bound to the current service session, workspace ID, and agent ID. Requests carry
those claims and the token. The server recomputes the token and compares it in
constant time, so changing either identifier invalidates the credential. A
valid agent can list and address only agents registered in the same workspace;
the authenticated sender identity is never accepted from message payload data.

Local agent processes receive:

- `OPENPODIUM_IPC_AVAILABLE=1`
- `OPENPODIUM_IPC_ENDPOINT`
- `OPENPODIUM_IPC_TOKEN`
- `OPENPODIUM_IPC_VERSIONS`
- `OPENPODIUM_WORKSPACE_ID`
- `OPENPODIUM_AGENT_ID`
- `OPENPODIUM_CLI`, containing the absolute path to the current executable

Remote and container environment wrappers receive the two identity values and
`OPENPODIUM_IPC_AVAILABLE=0`, but no endpoint or token. Local Shell agents and
custom presets are also unavailable because OpenPodium cannot safely inject an
automatic prompt into an arbitrary command. Host forwarding and custom delivery
strategies require an explicit transport design.

## Protocol versions

Every request identifies the protocol as `openpodium-ipc`, supplies all client
versions in preference order, a request ID, authentication claims, and one
tagged command. The server selects the first supported version. When no version
overlaps, it returns `incompatible_protocol` with the server's supported
versions and an actionable upgrade message.

A request envelope has this shape:

```json
{
  "protocol": "openpodium-ipc",
  "supported_versions": [2, 1],
  "request_id": "request-7d3464d3",
  "credentials": {
    "workspace_id": 4,
    "agent_id": 7,
    "token": "<injected capability token>"
  },
  "command": {
    "type": "list_agents"
  }
}
```

Command payloads are:

```json
{"type":"list_agents"}
{"type":"send_task","message_id":"task-1","recipient_agent_id":8,"title":"Review IPC","body":"Check the protocol implementation."}
{"type":"report_progress","message_id":"progress-1","task_message_id":"task-1","body":"Targeted tests are running."}
{"type":"respond","message_id":"response-1","task_message_id":"task-1","status":"completed","body":"Review complete."}
{"type":"send_handoff","message_id":"question-1","recipient_agent_id":8,"kind":"question","body":"Which API should I use?"}
{"type":"report_handoff_progress","message_id":"progress-2","handoff_message_id":"question-1","body":"Checking the domain model."}
{"type":"respond_to_handoff","message_id":"response-2","handoff_message_id":"question-1","status":"completed","body":"Use the workspace aggregate."}
{"type":"cancel_handoff","message_id":"cancel-1","handoff_message_id":"question-1","reason":"Answered elsewhere."}
```

`status` is `completed`, `failed`, or `blocked`. IDs contain 1–128 ASCII
letters, digits, dots, dashes, or underscores. Agent IDs are positive integers,
titles are limited to 255 characters, and message bodies to 32,768 characters.

A successful response echoes the protocol, selected version, and request ID,
then returns either an agent list or an acknowledgement:

```json
{"protocol":"openpodium-ipc","version":2,"request_id":"request-7d3464d3","result":{"type":"accepted","message_id":"task-1","duplicate":false}}
```

Failures omit `result` and return an error object. Stable protocol error codes are
`malformed_request`, `frame_too_large`, `incompatible_protocol`, `unauthorized`,
`invalid_request`, `agent_not_visible`, `idempotency_conflict`, and
`service_unavailable`. Compatibility failures also include
`supported_versions`.

Version 1 supports:

- `list_agents`: returns agent IDs, names, programs, lifecycle states, whether
  each entry is the caller, and its connection capabilities.
- `send_task`: publishes a task title and body to an agent in the caller's
  workspace.
- `report_progress`: publishes a progress body associated with an earlier task
  message.
- `respond`: publishes a successful, failed, or blocked response associated
  with an earlier task message.

Version 2 is preferred and adds typed `send_handoff`,
`report_handoff_progress`, `respond_to_handoff`, and `cancel_handoff` commands.
It distinguishes tasks from questions, supports parent-message correlation and
optional response deadlines, and generalizes progress and responses to either
handoff kind. Version 1 remains available to existing clients; orchestration
normalizes both versions into the same durable domain model.

Each structured message has a client-generated opaque ID. The server validates
IDs and text sizes before publishing. The first accepted `(workspace, message
ID)` durably records the authenticated sender, canonical payload, and
acknowledgement before success is returned. An identical retry by that sender
returns the acknowledgement without publishing twice; reuse by another sender
or with a different payload returns
`idempotency_conflict`. Pending messages and deduplication receipts survive an
OpenPodium restart; domain application is acknowledged separately after the
orchestrator has journaled the message.

Errors are tagged objects with a stable code, a human-readable message, and
optional structured details. Authentication failures deliberately do not reveal
whether a workspace, agent, or token component was wrong. Malformed or unknown
requests, oversized frames, invalid identifiers, invisible recipients, and
incompatible versions produce actionable errors.

## CLI

The existing `openpodium` executable remains the desktop application without
arguments and exposes these non-interactive commands:

```text
openpodium ipc agents list
openpodium ipc task send --to <agent-id> --title <title> --body <text> [--parent <handoff-id>] [--timeout-ms <milliseconds>] [--message-id <id>]
openpodium ipc question send --to <agent-id> --body <text> [--parent <handoff-id>] [--timeout-ms <milliseconds>] [--message-id <id>]
openpodium ipc progress report --handoff <message-id> --body <text> [--message-id <id>]
openpodium ipc respond --handoff <message-id> --status <completed|failed|blocked> --body <text> [--message-id <id>]
openpodium ipc cancel --handoff <message-id> --reason <text> [--message-id <id>]
```

Connection and identity values default to the injected environment variables.
Successful commands write one JSON response to standard output. Usage,
connection, authentication, compatibility, and server errors go to standard
error with distinct non-zero exit statuses. Tokens are accepted only through
the environment and are never command arguments.

## Verification

Protocol tests cover serialization, negotiation, validation, and stable error
codes. Temporary-directory service tests prove secret-file permissions where
supported, invalid-token rejection, cross-workspace isolation, exact retry
deduplication, conflicting ID reuse, concurrent clients, malformed input,
oversized frames, durable restart recovery, unavailable recipient adapters, and
clean shutdown. CLI tests exercise argument parsing and environment discovery
without opening the desktop window. Adapter tests verify that supported local
launches receive scoped connection metadata while remote and unsafe launches do
not receive the endpoint or token.
