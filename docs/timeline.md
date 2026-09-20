# Orchestration timeline and recovery

Status: Issue #14 implementation contract
Updated: 2026-09-19

## Boundary

The timeline is a read model of the append-only domain journal. It never infers
task or agent state from terminal output, transient UI flags, or notification
delivery. The application shell owns rendering, desktop notifications, and
navigation; the orchestrator owns recovery commands; persistence owns ordered
event retrieval.

## Timeline and attention

The application presents all workspace events in journal order and can narrow
the view to one task. Task-scoped entries include task creation and lifecycle
changes plus delivery attempts, progress, responses, cancellation, and timeout
changes from the task's handoff. Agent lifecycle entries make waiting and
stopped agents visible in the chronological view.

Current task and agent labels come from the recovered workspace aggregate,
whose state is itself produced by replaying the same durable events. Blocked
tasks and failed tasks without a subsequent retry are urgent; completed tasks
are informational. The workspace sidebar shows counts for current urgent
states.

On startup, the application records the journal high-water mark before watching
for attention events, so recovery does not replay old desktop notifications.
New blocked, failed, and completed task transitions create a desktop
notification. Activating it uses a typed target containing the workspace, task,
and best available node. A task node is preferred; otherwise the assignee's
agent node is selected. Activation switches workspace, selects that node, and
opens the task-specific timeline.

## Recovery actions

- **Inspect** navigates to the task target and filters the timeline without
  changing durable state.
- **Cancel** is available while a task is queued, delivered, running, or
  blocked. It records handoff cancellation and the task transition, removes
  pending work, and delivers the cancellation to the recipient when possible.
- **Resume** is available only while blocked. It creates a durable continuation
  handoff for the same task, records the valid `Blocked -> Running` transition,
  and preserves the terminal handoff that reported the block.
- **Retry** is available only while failed or cancelled. It atomically creates
  a new queued task and handoff. The new task points to the previous task with
  `retry_of`, and the old attempt is never changed or removed.

Recovery actions are derived from the recovered task state, so restarting the
application restores the same available controls. The orchestrator rebuilds
its pending delivery queues from durable handoffs before the first UI update.

## Verification

Targeted tests cover ordered journal reads, task-scoped projections, attention
classification, retry history, cancel and resume invariants, notification
targets, navigation across workspaces, and recovery-action availability after
reopening the database.
