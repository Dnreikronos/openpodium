# Local supervisor and summaries

Status: Issue #22 implementation contract
Updated: 2026-09-20

## Boundary

The supervisor is a read model over durable workspace timelines and local Git
collision observations. It may recommend an action, but it never executes one.
Agent execution, orchestration, persistence, and Git integration continue when
summary generation or desktop notification delivery fails.

The compact supervisor surface is rendered in the application sidebar so it
remains visible outside the active canvas. It aggregates every loaded
workspace, not only the selected one.

## Evidence-linked claims

Rule-based summaries group attention, completion, failure, and collision
signals by workspace. Every claim contains one or more typed evidence links:

- timeline evidence identifies the workspace and durable timeline event;
- collision evidence identifies the workspace, overlapping path, and both
  checkouts from the local collision scan.

The interface renders those references with each claim. Claims without
evidence are rejected by construction.

Recommendations are derived from the same signals. They are descriptive only:
inspect blocked work, retry or inspect failures, review collisions, or continue
when no intervention is required.

## Optional model adapters and privacy

Rule-based summaries are the default and require no model. An optional adapter
must declare whether it is on-device or may send supplied project content to an
external service. External adapters are refused unless the user configuration
explicitly allows project content to leave the machine.

Adapters receive the already evidence-linked rule summary. Adapter errors,
invalid empty output, and privacy-policy rejection all return the rule summary
and an advisory error; they never block or alter agent work.

## Notifications

Desktop notifications are classified as attention, completion, failure, or
collision. Each class has an independent enabled flag and cooldown. The local
rate limiter keys suppression by workspace and class so one noisy workspace
does not hide another workspace's event.

Notification activation navigates when the source has a task target. Delivery
failure or rate limiting changes no durable state.

## Verification

Targeted tests cover cross-workspace aggregation, evidence on every claim,
collision evidence, recommendations, adapter privacy and fallback behavior,
and independent per-class/per-workspace notification rate limits.
