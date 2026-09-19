# Typed handoff orchestration

Status: Issue #13 implementation contract
Updated: 2026-09-19

## Boundary

The orchestration module turns authenticated IPC messages into durable workspace
handoffs, serializes delivery to each receiving agent, and records every prompt
delivery attempt. The IPC service owns authentication, protocol negotiation,
transport receipts, and retry deduplication. The domain owns handoffs, tasks,
responses, progress, cancellation, deadlines, and their invariants. The runtime
owns PTY input and does not infer orchestration state from terminal output.

Issue #13 does not add timeline, notification, retry, or recovery controls to
the user interface. Those views and actions are owned by issue #14, using the
durable events introduced here.

## Protocol compatibility

Protocol version 2 generalizes task-specific messages into typed handoffs:

```json
{"type":"send_handoff","message_id":"work-1","recipient_agent_id":8,"kind":"task","title":"Review IPC","body":"Check the protocol.","parent_message_id":null,"response_timeout_ms":null}
{"type":"report_handoff_progress","message_id":"progress-1","handoff_message_id":"work-1","body":"Running targeted tests."}
{"type":"respond_to_handoff","message_id":"response-1","handoff_message_id":"work-1","status":"completed","body":"Review complete."}
{"type":"cancel_handoff","message_id":"cancel-1","handoff_message_id":"work-1","reason":"No longer needed."}
```

`kind` is `task` or `question`. Tasks require a non-empty title; questions do
not. A child handoff may name an existing `parent_message_id`. Agent cycles are
valid because an agent may ask the originator a follow-up question. The parent
chain is immutable and limited to 16 handoffs.

Version 1 remains accepted. Its `send_task`, `report_progress`, and `respond`
commands are translated to the corresponding version 2 semantics. New clients
offer versions 2 and 1 in preference order. Questions and cancellation require
version 2.

An acknowledgement means the transport receipt and idempotency key are durable;
it does not mean the recipient has received or completed the handoff. Reusing a
message ID with a different sender or payload remains an idempotency conflict.

## Durable state

A handoff records its stable protocol message ID, source and recipient agents,
typed payload, optional parent, creation time, optional response deadline,
delivery attempts, progress entries, response, cancellation, and timeout.
Task handoffs create the task and handoff atomically. The task lifecycle remains
the authority for work state; handoff delivery and conversation records do not
duplicate that lifecycle.

Every delivery attempt records:

- its ordinal and timestamp;
- the selected adapter mechanism;
- whether it started, delivered, or failed;
- an actionable failure reason when applicable.

The transport inbox and domain application marker make restart recovery
idempotent. A crash after an accepted IPC request cannot lose the request. A
crash during PTY input leaves a started attempt whose outcome is unknown; it is
failed during recovery instead of silently claiming successful delivery.

## Delivery

Each agent has a logical mailbox. Handoffs to the same recipient are delivered
in acceptance order and never write concurrently to its PTY. Different
recipients may progress independently.

Codex, Claude Code, and OpenCode each select a typed terminal delivery mechanism
that is stored on every attempt. Their interactive adapters currently share the
same safe operation: a labelled prompt through bracketed paste followed by one
submit key. Delivery addresses the terminal by workspace and agent identity,
never by focused canvas node, so focus cannot change the recipient. Control
bytes are not written as terminal keystrokes.

Shell agents do not accept automatic handoffs because pasted text would be
executed as shell input. Custom presets also default to unavailable until an
explicit safe delivery strategy is added. Capability discovery reports this
distinction.

If the receiving terminal is starting or temporarily unavailable, delivery is
retried with bounded backoff for at most 30 seconds. Each failed write is an
attempt. Exhaustion records a delivery failure; it does not overwrite or delete
the handoff.

## Progress, responses, cancellation, and timeouts

Only the recipient can report progress or respond. Progress moves a delivered
task to running on its first report. A completed, failed, or blocked response
updates the task through valid lifecycle transitions and stores the response.
Duplicate protocol retries do not append duplicate progress or responses.

The originating agent may cancel its handoff. A user-facing cancellation path
may cancel any handoff when issue #14 adds recovery controls. Cancellation is
terminal and late progress or responses are rejected predictably.

Delivery has the fixed 30-second deadline described above. Work has no arbitrary
default deadline. A sender may supply `response_timeout_ms`; expiration records
a timeout and transitions an unfinished task to failed. Because the orchestrator
never blocks a parent while waiting for a child, agent cycles cannot deadlock.
At the chain-depth limit, a child request is rejected with an actionable error.

## Verification

Targeted tests cover protocol v1/v2 negotiation and translation, durable
idempotency across service restart, atomic task/handoff creation, parent-chain
validation, cancellation authority, response deadlines, per-recipient ordering,
focus-independent delivery, unavailable and unsafe adapters, attempt history,
concurrent senders, retries, and recovery of interrupted attempts.
