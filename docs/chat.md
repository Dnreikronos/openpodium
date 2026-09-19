# Agent chat threads and prompt composer

Status: Issue #11 implementation contract
Updated: 2026-09-19

## Boundary

Chat threads are durable workspace state owned by an agent. A thread has a
stable identifier, name, display color, ordered messages, and one composer
draft. Messages, draft text, draft attachments, and draft mentions are written
through the existing domain journal so reopening the application restores the
same conversation state.

The chat surface does not infer a structured conversation by scraping terminal
escape sequences. Submitting a prompt records the user message first and, when
the selected agent has a live terminal, sends the same text to that terminal as
one input operation. Structured incoming messages will use the same domain
message command when the IPC work in issue #12 is available. Typed handoff
routing remains owned by issue #13.

The selected thread and the chat/terminal view are session UI state. Switching
views never stops a terminal, discards its emulator state, or replaces the
durable thread draft.

## Durable model

- Thread names use the existing validated `Name` type. Colors use a validated
  `#RRGGBB` value.
- A thread belongs to exactly one agent. Messages and attachments belong to
  exactly one thread and cannot be moved between threads.
- Message authors are the local user or the thread's owning agent. User
  messages are created atomically from the current draft; the same event clears
  that draft. Agent messages never clear a user draft.
- A draft may be empty, but submitted prompt text may not. Draft updates are
  journaled as they are edited so clean shutdown is not required for recovery.
- Mentions target agents, tasks, or handoffs that have a direct canvas
  connection to the owning agent's node. The domain revalidates those
  references when a draft or message is written.
- Threads are append-only in this slice. They may be renamed or recolored, but
  deleting conversation history is deliberately outside issue #11.

## Attachments and file references

Imported attachments are copied into OpenPodium's application-data directory,
never into the project and never to a remote service. Durable records contain a
generated storage key, original display name, media type, and byte length; they
never treat an arbitrary path from message content as trusted storage.

The initial limits are 10 MiB per attachment, 25 MiB total per draft, and eight
attachments per draft. A rejected import does not change domain state. A copy
is staged and renamed before its attachment record is journaled; a failed
journal write removes the staged copy. Unreferenced files left by process
termination are harmless and may be collected by a later maintenance pass.

Markdown images render only from attachment identifiers already owned by the
thread. HTTP(S), data, and arbitrary filesystem image sources are not fetched.
File references resolve relative to the canonical workspace root and are
displayed only when canonicalization keeps them inside that root. A file
reference is an application action, not a shell command.

## Safe rich rendering

Messages are parsed into Iced-native widgets. Raw HTML is ignored, scripts are
never evaluated, and Markdown cannot instantiate a web view. Links are
actionable only for `http` and `https`; all other schemes render without an
open action except the explicit local attachment and workspace-file forms
described above.

Standard Markdown includes headings, emphasis, lists, quotes, code blocks, and
tables. Fenced `mermaid` blocks support a bounded flowchart subset (`graph` or
`flowchart`, `TD` or `LR`, labelled nodes, and directed edges). The renderer
ignores Mermaid directives, HTML labels, click handlers, styles, and URLs;
unsupported input falls back to a code block. Diagrams are limited to 64 nodes,
128 edges, and the same message-size limit as other content.

Image previews are restricted to locally imported PNG, JPEG, GIF, or WebP
attachments within the attachment byte limit. Other attachments render as a
file card with name, type, and size.

## Composer and runtime behavior

The selected agent exposes Chat and Terminal controls in its inspector. Chat
shows thread selection, creation and appearance controls, a scrollable message
history, attachment and mention summaries, and the composer. Terminal keeps the
existing start, reconnect, and stop actions visible.

Submitting a non-empty draft commits the durable message before attempting PTY
input. If no terminal is running, the message remains recorded and the UI
reports that delivery is pending manual action. If PTY input fails, the message
also remains recorded and the error is visible; retries must not create a
second message implicitly. The raw terminal stays live and selectable while an
agent turn is running.

## Verification

Targeted domain and recovery tests cover thread ownership, connected mentions,
atomic draft submission, snapshot/event round trips, and restart restoration.
Renderer tests cover unsafe links and image sources, raw HTML, tables, code,
file-reference containment, Mermaid limits, and fallback behavior. Attachment
tests use temporary directories to prove size/count limits, local copying, and
cleanup after a rejected journal write. Application tests verify that view
switching preserves the terminal session and the active draft.
