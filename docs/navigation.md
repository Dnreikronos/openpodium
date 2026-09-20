# Global search, command palette, and keyboard navigation

Issue: https://github.com/Dnreikronos/openpodium/issues/18

## Boundary

Navigation is application behavior over durable workspace state. The search
index is a derived, replaceable read model; it is never journaled and never
becomes a second source of truth. The dependency-free navigation module owns
search documents, fuzzy ranking, command metadata, shortcut validation, and
deterministic graph traversal. The application shell owns asynchronous index
refresh, keyboard event routing, palette presentation, and applying typed
navigation targets.

## Indexed content

Every loaded workspace contributes its name and project directory, agents,
tasks, chat threads and messages, and canvas nodes. Project checkouts and
file-backed canvas nodes contribute normalized project-relative paths. Note files contribute their
bounded UTF-8 body when their floor checkout is available. Text nodes
contribute their Markdown body. Arbitrary repository file contents are not
indexed.

Each document has a stable key and a typed target containing the workspace,
optional floor, optional node, and the content identity needed to restore the
result. Chat targets retain the agent, thread, message, and match offset. Note
and text targets retain the node and match offset. Results never use display
labels as identities.

Index construction runs away from the UI thread. A workspace revision marks
only that workspace stale; rebuilding it replaces that workspace's documents
atomically while indexes for other workspaces remain usable. Querying uses the
last completed index and therefore never waits for filesystem reads or domain
writes.

## Search and commands

The palette opens in search mode and fuzzy-matches case-insensitive Unicode
text. Consecutive and word-boundary matches rank above sparse subsequences;
stable document keys break ties. Prefixing the query with `>` switches to
command discovery. Empty command queries list all commands.

Every executable command is registered once with a stable identifier, label,
current shortcut, and handler message. The palette and keyboard dispatcher use
the same registry so displayed shortcuts cannot drift from behavior.

Selecting a search result performs, in order:

1. switch to the result workspace;
2. switch to the result floor when it has one;
3. select and center the result node when it has one;
4. open the relevant chat thread, task timeline, note, or text surface;
5. restore the nearest supported content position and preserve its match for
   visual emphasis.

## Shortcuts

Bindings are application-global and stored locally in the application SQLite
database. Defaults are platform-aware (`Cmd` on macOS and `Ctrl` elsewhere).
Rebinding rejects duplicate active chords. Every command remains discoverable
from the palette even when unbound. Focused text inputs keep ordinary editing
chords; the palette and canvas-focus commands remain globally available.

The initial registry includes opening the palette, cycling workspaces, jumping
between agents needing attention, traversing nodes and connections, zooming,
resetting the view, and moving focus between the canvas and its active content.
Existing canvas edit commands are routed through the registry. A focused
terminal receives ordinary keystrokes, while the palette shortcut and the
existing terminal-focus release shortcut remain application commands.

Traversal order is deterministic. Node traversal follows `(y, x, id)` canvas
order. Connection traversal visits outgoing neighbors by connection kind then
node identifier, followed by incoming neighbors in the same order. Repeated
navigation wraps.

## Persistence and compatibility

Shortcut overrides extend application settings rather than workspace domain
state. Database migration is additive and preserves existing journals and
snapshots. Unknown command identifiers are retained but ignored, allowing a
newer build's settings to survive an older binary. Invalid or conflicting
stored bindings fall back to defaults and surface an actionable warning.

## Verification

Targeted tests cover fuzzy ranking and stable ties, Unicode content offsets,
index replacement by workspace, Git and non-Git project paths, deterministic
graph traversal, command discovery, binding conflict detection, shortcut
persistence and migration, keyboard dispatch while terminals are focused, and
navigation that restores workspace, floor, node, and content identity.
