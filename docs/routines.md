# Routines, triggers, and repeatable runs

Status: Issue #21 implementation contract  
Updated: 2026-09-20

## Boundary

The routine module is a durable scheduler above handoff orchestration. It owns
dependency readiness, approvals, output binding, resource reservations, and
interruption handling. The task lifecycle stays authoritative for work state,
handoff delivery stays with the orchestrator, and the IPC service keeps owning
authentication, transport receipts, and idempotency.

"Deterministic" here means the orchestration decisions and the recorded inputs.
Given the same journal, a recovered scheduler makes the same choices a live one
did. Agent responses still vary; nothing pretends otherwise.

## Immutable versions

A routine is a name plus an append-only list of versions. Editing a routine
appends a version; it never rewrites one. Each version declares its inputs and a
validated DAG of steps.

Every step declares:

- the agent it binds;
- a prompt whose `{{name}}` placeholders must all be bound;
- its direct dependencies;
- where each binding takes its value from — a run input, an earlier step's
  declared output, or a literal;
- the outputs it must return;
- whether it needs approval before dispatch;
- how many attempts it may use;
- the checkout and named resources it claims.

A version is rejected before it is stored when steps form a cycle, when a
dependency does not exist, when a binding reads an input the routine does not
declare, when it reads an output the producing step does not declare, or when it
reads an output from a step it does not transitively depend on. Reading an
output without depending on its producer has no defined order, so it is refused
even though the output exists.

## Pinning

Starting a run freezes the version, the resolved inputs, the agent binding and
canonical checkout for every step, the Git revision each of those checkouts was
on, and the canvas template. A routine edited a second later does not change
what a running run does.

A version may carry a canvas template. Starting such a run instantiates it with
fresh entity identifiers and binds the run's steps to the agents that
instantiation created, so two runs of the same routine never share an
arrangement.

## Persistence before dispatch

`DomainCommand::DispatchRoutineStep` records the attempt, its task, its handoff,
and the reservations it acquires in one journal batch. Only after that
transaction commits is anything submitted for delivery.

Crashing between the journal write and delivery leaves a dispatched step whose
outcome is unknown. Crashing before the write leaves a step that is simply still
ready. Neither loses work and neither claims work happened.

Retries append an attempt. The previous task and handoff stay exactly as they
were, and the new task points at the previous one through `retry_of`.

## Scheduling and reservations

The scheduler runs on the application's existing orchestration tick. It selects
ready steps in a stable order — runs oldest first, then steps by identifier —
and acquires a step's declared claims together. A step that cannot take every
claim waits instead of starting half-isolated.

A claim is one of:

- the bound agent;
- the canonical checkout the step resolved to;
- a named resource the step declared.

Reservations span every active run, so two runs cannot write to the same
checkout at once. Checkout identity is canonicalized at the filesystem
boundary, so two spellings of the same path conflict.

Routine work is submitted through an internal path, not by impersonating an
agent. A handoff records a typed origin: an agent, or a run and step.
Mailboxes continue to serialize prompt writes; the reservation, not the
mailbox, is what keeps a whole step exclusive.

## Completion

A step completes only on a typed IPC response. Terminal text is never read.
`openpodium ipc respond` carries `--output key=value` pairs (protocol version 4),
and a step that declares outputs but completes without them is failed with an
actionable reason rather than given an invented value. Declared outputs are
validated and stored before any dependent step is released.

## Recovery and cancellation

Runs, attempts, and reservations are reconstructed from durable events.

- Steps that were never dispatched become ready again.
- Steps that were dispatched have an unknown outcome and are marked interrupted.
  They keep their reservations until a user confirms the work stopped.
- Completed outputs and pending approvals survive untouched.

Cancellation stops further dispatch and asks the orchestrator to cancel active
work. A cancelled handoff does not prove the agent stopped, so dispatched steps
become interrupted rather than cancelled and keep holding conflicting
reservations until the user resolves them.

## Triggers

| Trigger | Behaviour |
| --- | --- |
| Manual | Validates inputs and starts the pinned version |
| Filesystem | Filters paths, debounces changes, and identifies an occurrence by the observed state |
| Git | Observes local ref changes and records the before and after revisions |
| Schedule | Persists the cadence, UTC offset, timezone label, next occurrence, enabled state, and firing identity |

Consuming a trigger occurrence and creating its run happen in the same
transaction, so a duplicate observation cannot produce a second run. A routine
has one active run at a time; a repeated trigger coalesces onto the run already
in flight.

Schedules run only while OpenPodium is open. Occurrences that pass while it is
closed are skipped, counted, and shown on the trigger rather than replayed in a
burst at startup. OpenPodium carries no timezone database, so a schedule stores
the fixed UTC offset the user chose plus a label for display.

## Interface

The routines panel renders routine editing, input entry, trigger toggles, next
occurrence, and run inspection from journal-backed projections, with approval,
resume, and cancel controls. Routine approval is a scheduling gate; it does not
touch portal permissions, which every portal action still passes through.

## Verification

Targeted tests cover version pinning across a later edit, sequential dependency
and output binding, completion without a declared output, conflicting resource
claims across concurrent steps, the crash boundary around dispatch, retry
history, cancellation races, approval gating, duplicate trigger delivery,
filesystem debouncing and deduplication, schedule skipping and disabling,
routine DAG validation, template instantiation with fresh identifiers, and
recovery of a database written before routines existed.
