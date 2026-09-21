# OpenPodium architecture

Status: Initial direction  
Updated: 2026-09-18

## Architectural drivers

OpenPodium must keep interactive terminals responsive while several subprocesses, Git operations, persistence writes, and canvas updates occur concurrently. It must also recover meaningful state without treating terminal scrollback as the source of truth.

The design therefore separates the durable domain model from terminal rendering and UI state. UI actions and runtime observations become explicit domain commands and events.

## Technology choices

- **Rust stable** for the application and domain logic.
- **Iced 0.14 + WGPU** for a cross-platform, GPU-rendered desktop UI.
- **Tokio** for runtime concurrency and process coordination.
- **portable-pty** for the native PTY abstraction.
- **alacritty_terminal** for terminal emulation state and escape-sequence parsing.
- **SQLite** in WAL mode for workspace metadata, tasks, and the event journal.
- **Git CLI boundary** for worktree operations initially, keeping behavior aligned with the user's installed Git.

Dependencies are introduced only when their capability is implemented. This document records direction, not a mandate to add unused crates.

## Component boundaries

### Application shell

Owns windows, commands, keyboard mappings, notifications, and top-level dependency wiring. It translates UI messages into domain commands and renders immutable view models.

### Canvas

Owns camera transforms, node geometry, selection, connection hit-testing, and spatial interaction. Canvas coordinates remain independent of screen coordinates so pan and zoom do not mutate node positions.

### Domain

Owns workspaces, agents, roles, tasks, handoffs, and lifecycle transition rules. It contains no Iced, PTY, SQLite, or operating-system types and is testable without a desktop session.

### Runtime

Owns PTYs, agent adapter processes, input/output streams, cancellation, and runtime health. It publishes structured observations but cannot directly mutate domain state.

### Routine scheduler

Owns immutable routine versions, their triggers, and their runs. It decides
which steps are ready, holds the resources a step declared for as long as it
executes, and records every dispatch before the work leaves the process. It
submits work to the orchestrator through an internal path rather than
impersonating an agent.

### Orchestrator

Validates and routes typed handoffs. A local OpenPodium command/IPC endpoint will allow agents to list peers, send tasks, report progress, and return responses. PTY text injection may be an adapter mechanism, but it is not the durable messaging protocol.

### Persistence

Stores current snapshots plus an append-only journal of meaningful changes. Transactions update the journal and materialized state together. Canvas movement may be coalesced before persistence; task and lifecycle events are never silently discarded.

### Git isolation

Creates and inventories worktrees, records branch ownership, calculates changed paths, and reports collisions. It proposes integration operations but never merges or discards user work without an explicit action.

## State and event flow

```text
User or runtime observation
          |
          v
     Domain command
          |
     validate/decide
          |
          v
      Domain event -----> SQLite journal
          |                    |
          v                    v
    in-memory state       recovery/replay
          |
          v
       view model
          |
          v
        Iced UI
```

The domain state is authoritative. Terminal output is evidence and user-facing content, not the only record that a task exists or completed.

## Initial code organization

The repository begins as one package with focused modules while boundaries are small and changing:

```text
src/
  main.rs       application entry point
  app.rs        application shell and messages
  canvas.rs     camera, nodes, and interaction
  domain.rs     public facade for dependency-free domain types
  domain/
    ids.rs            typed entity identifiers
    value_objects.rs  validated names, content, geometry, and timestamps
    lifecycle.rs      agent and task transition rules
    entities.rs       workspace-owned domain entities
    events.rs         commands, events, timeline records, and errors
    workspace.rs      aggregate validation and event application
    tests.rs          exhaustive domain behavior tests
```

Runtime, persistence, orchestration, and Git modules will be added as their vertical slices begin. A module becomes a workspace crate when it needs an independently testable dependency boundary or separate platform implementation; the project will not start with empty architectural crates.

## Reliability rules

- No task status change bypasses domain transition validation.
- Runtime handles are ephemeral and never serialized.
- Every subprocess has an owned cancellation path and an observable exit status.
- Writes use transactions; recovery must tolerate termination between any two events.
- User project files are never deleted as cleanup.
- Worktree integration is explicit and reversible wherever Git allows it.

## Security and privacy

- Bind IPC to the local user session and authenticate it with a per-install secret.
- Treat terminal output, repository content, and agent messages as untrusted input.
- Never interpolate user text into a shell command; use argument arrays.
- Redact likely secrets from diagnostic logs.
- Disable telemetry by design rather than by a preference toggle.
