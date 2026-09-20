# Canvas context and drawing nodes

Issue: https://github.com/Dnreikronos/openpodium/issues/17

## Boundary

Canvas nodes may either reference an existing workspace entity or own durable
canvas content. Agent, task, and handoff nodes keep their current references.
Notes, file trees, file artifacts, diffs, text, shapes, arrows, and freehand
drawings are first-class node content and survive restart, duplication,
clipboard transfer, import/export, grouping, connections, and undo/redo.

The dependency-free domain owns durable node content and validates portable
paths, drawing geometry, and style values. Filesystem traversal, text decoding,
Git commands, file watching, and rendering remain at the application boundary.
Derived file contents, directory entries, Git status, and rendered diffs are
never copied into the domain journal.

## Project paths

Every file-backed node stores a normalized path relative to its floor checkout.
Absolute paths, parent traversal, empty components, and paths that escape after
symlink resolution are rejected before access. The same durable path therefore
resolves independently in the original checkout and in managed worktree floors.

New notes default to `.openpodium/notes/<stable-node-id>.md` in the active
checkout. Creating a note is an explicit user action and may create the
`.openpodium/notes` directory. OpenPodium does not edit `.gitignore`; users may
track or ignore these ordinary Markdown files as they prefer.

## Notes and external changes

A note node stores its project path and display title. Its Markdown body lives
only in the referenced file so there is one source of truth for agents and the
UI. Local agents receive the resolved note paths for directly connected notes
as a JSON string array in `OPENPODIUM_CONNECTED_NOTES`. Agents read and update
those ordinary files with their normal filesystem tools. Remote environments do
not receive local paths.

The editor keeps a transient buffer with the file revision observed when it was
loaded. A clean buffer reloads after an external change. A dirty buffer is never
replaced: the node shows a conflict and offers reload-discard or explicit
overwrite. Saving uses a temporary sibling file followed by an atomic rename
and fails if the observed revision changed. Missing files remain represented
and can be recreated explicitly.

## File trees, previews, diffs, and artifacts

A file-tree node stores a project-relative root directory. Children are loaded
on demand, sorted deterministically, and refreshed outside the UI thread.
Ignored entries are hidden by default. Each entry may display the Git change
kind already produced by the collision inventory. Opening a text file creates
or focuses a file artifact; requesting its changes creates or focuses a diff.

A file artifact is a live reference to one project-relative file. Text preview
is allowed only after bounded byte inspection rejects NUL-containing data and
strict UTF-8 decoding succeeds. Known supported image formats may use the
existing decoded-image path. Other binary files show metadata and an external
open action; they are never decoded as text. Preview reads have explicit size
limits and do not block the UI thread.

A diff node stores a path and a durable comparison mode, initially working tree
against `HEAD`. Git produces the rendered text on demand. Untracked text files
render as additions; binary changes render metadata only. The node shows stale
or unavailable state when its checkout, revision, or path cannot be resolved.

Agent-produced files become artifacts by entering their project-relative path
or selecting that path from the project context controls. Only an existing path
within the active checkout is accepted, and the artifact is created through the
normal journal boundary.

## Drawing content

Text nodes own Markdown text. Shape nodes support rectangles and ellipses with
validated fill, stroke, and stroke width. Arrow nodes own two normalized local
endpoints plus stroke and optional label; they are visual annotations, not
semantic canvas connections. Freehand nodes own a bounded list of normalized
local points and a validated stroke. Resizing maps normalized drawing geometry
without duplicating layout state.

Drawing edits use the same preview-then-commit rule as node movement: pointer
motion stays transient and a completed gesture produces one durable canvas
replacement. Text editing commits one logical edit when focus leaves the node
or the user explicitly saves it.

## Connections and portable fragments

Existing typed agent/task/handoff connections keep their semantics. Context and
drawing nodes use a neutral reference connection when either endpoint has no
workspace-entity relationship. Self-connections and duplicate directed pairs
remain invalid.

Canvas copy serializes a versioned fragment containing selected nodes, complete
selected groups, and connections whose two endpoints are selected. Paste
allocates fresh identifiers and offsets geometry. File paths stay relative;
pasting into another workspace preserves the node but marks unavailable paths
instead of reading outside that workspace. The same fragment codec is the
issue-17 import/export boundary and will be reused by workspace templates in
issue #19.

Duplicating a file-backed node duplicates the reference, not the underlying
file. Duplicating a drawing or text node copies its owned content. Undo and redo
apply complete before/after canvas snapshots, including node content, exactly
as existing layout edits do.

## Persistence and compatibility

Node content is encoded in new event and snapshot format versions. Older
Agent/Task/Handoff records decode into the corresponding reference variants.
Unknown variants fail with an actionable compatibility error and never replace
the last valid snapshot. No filesystem content is embedded in snapshots or
events.

## Verification

Targeted tests cover:

- node content validation and reference semantics;
- event, snapshot, and portable-fragment round trips;
- backward decoding of existing agent, task, and handoff nodes;
- path normalization, symlink escape rejection, bounded previews, and atomic
  compare-and-swap note writes;
- ignored and tracked path detection plus text, untracked, and binary diffs;
- drawing validation, normalized geometry, hit testing, and editor history.

Each implementation phase runs formatting, the compiler, Clippy, and only the
tests targeted to the changed behavior.
