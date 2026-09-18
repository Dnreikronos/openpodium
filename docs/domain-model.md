# OpenPodium domain model

Status: Issue #2 implementation contract  
Updated: 2026-09-18

## Boundary

The domain module owns durable workspace, role, agent, task, handoff, node, and
timeline concepts. It uses only Rust standard-library types. UI, PTY, database,
filesystem, and operating-system values are translated at the application
boundary.

`Workspace` is the aggregate boundary. A caller submits a `DomainCommand`; the
workspace validates it, applies the resulting `DomainEvent`, and returns that
event for journaling. The same event-application path is public so persisted
events can be replayed without bypassing domain invariants.

All entity identifiers are distinct newtypes. Names and free-form content are
validated value objects. Canvas positions and sizes reject non-finite values,
and sizes must be positive.

## Agent lifecycle

| Current | Allowed next states |
| --- | --- |
| Starting | Running, Failed, Stopped |
| Running | Waiting, Completed, Failed, Stopped |
| Waiting | Running, Completed, Failed, Stopped |
| Completed | None |
| Failed | None |
| Stopped | None |

## Task lifecycle

| Current | Allowed next states |
| --- | --- |
| Queued | Delivered, Cancelled |
| Delivered | Running, Failed, Cancelled |
| Running | Blocked, Completed, Failed, Cancelled |
| Blocked | Running, Failed, Cancelled |
| Completed | None |
| Failed | None |
| Cancelled | None |

Same-state changes are invalid. Terminal states are immutable. A retry is a new
task whose `retry_of` field points to a failed or cancelled task, preserving the
previous attempt and its timeline.

## Consistency rules

- Referenced roles, agents, tasks, handoffs, and node targets must already
  exist in the workspace.
- A handoff must have different source and recipient agents.
- A task retry must reference a failed or cancelled task.
- An event that describes a different prior lifecycle state is rejected during
  replay instead of silently corrupting the aggregate.
- Validation completes before mutation, so every rejected command or event
  leaves workspace state unchanged.

Errors retain entity identifiers, field names, current states, and requested
states so the application can present actionable feedback without parsing an
error string.
