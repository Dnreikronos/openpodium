# Canvas editing contract

Status: Issue #6 implementation contract  
Updated: 2026-09-18

## Agent creation

The canvas offers three agent choices: Codex, Claude, and Shell. Creating one
adds a durable agent and a target-backed canvas node. The node reserves its body
for a terminal, but process launch and terminal emulation remain owned by issues
#7 and #8.

Agent names and identifiers are allocated within the active workspace. New
nodes are placed near the camera focus, offset when necessary to avoid exact
overlap, and raised above existing nodes.

## Editing behavior

- Clicking a node selects it. Shift-click toggles nodes in the selection.
- Dragging a selected node moves the selection in world coordinates. Members of
  the same group move together.
- The bottom-right handle resizes one node. Sizes have deterministic minimums.
- Positions and sizes snap to a 20-world-unit grid when a gesture is committed.
- Toolbar actions duplicate, remove, align, distribute, group, ungroup, and
  change z-order for the current selection.
- Groups are flat. A node belongs to at most one group. Removing a group keeps
  its nodes, while removing a node also removes incident connections and prunes
  groups that no longer contain at least two nodes.

## Connections

Connections are directed, visible, and have a kind determined by their endpoint
targets.

| Source | Target | Kind |
| --- | --- | --- |
| Agent | Agent | Coordination |
| Agent | Task | Assignment |
| Task | Task | Dependency |
| Agent, Task, or Handoff | Handoff | Handoff |
| Handoff | Agent or Task | Handoff |

Self-connections, duplicate endpoint pairs, and combinations outside this table
are rejected. These connections describe the canvas graph only. Durable task
delivery and orchestration remain owned by issue #13.

## Persistence and history

Selection, gesture previews, and undo stacks are session-local UI state. Each
completed gesture or toolbar action produces one durable canvas edit through
the workspace persistence boundary. Pointer movement is previewed in memory and
is not journaled frame by frame.

Every canvas edit records its before and after graph snapshots. Undo applies the
before snapshot as a new durable event; redo applies the after snapshot. This
means restart recovery always sees the layout currently visible to the user.
Switching workspaces clears transient selection and history.

Node deletion is one atomic canvas edit. Its snapshot transition removes the
node, incident connections, and invalid group memberships together, so recovery
cannot produce dangling references.
