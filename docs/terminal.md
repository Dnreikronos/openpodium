# Interactive terminal node contract

Status: Issue #8 implementation contract  
Updated: 2026-09-18

## Session lifecycle

Terminal sessions are ephemeral application state keyed by agent node ID. Creating an agent node
starts its selected program immediately in the workspace working directory. Restored nodes remain
offline until the user starts them, so reopening OpenPodium never launches commands unexpectedly.
Switching workspaces detaches the terminal view while its workspace-keyed process continues in the
background. Removing a node, stopping a terminal, or closing the application cancels its process
through the runtime boundary.

Reopening replays a node's last screen. Each session keeps a bounded tail of
the raw output it received, and that transcript is written to a side table on a
slow cadence and again as the application closes. On load the bytes are fed to a
fresh emulator, so a restored node shows the output it had, with its colours and
layout, while reporting `offline`. The process is gone; nothing restarts until
the user starts it.

A transcript is a display cache, never domain truth. It is capped so only the
tail survives, trimmed at a line boundary so a replay cannot begin partway
through an escape sequence, dropped when its node no longer exists, and safe to
delete at any time without losing anything the journal owns. Conversation
history is unaffected by any of this: chat threads, messages, and drafts are
journal-backed and restore on their own.

Shell nodes launch the user's login shell on Unix and `COMSPEC` on Windows. Codex and Claude nodes
launch the `codex` and `claude` executables. Every child receives `TERM=xterm-256color`,
`COLORTERM=truecolor`, and `TERM_PROGRAM=OpenPodium`. Custom agent commands and OpenCode presets
remain follow-up work; issue #9 custom environments are argv-based wrappers around these programs.

The node header reports `offline`, `starting`, `running`, `exited`, `stopped`, or `failed`. Runtime
handles and live emulator state are never serialized; only the bounded output transcript above is.

## Emulation and rendering

`alacritty_terminal` owns escape-sequence parsing, the primary and alternate grids, cursor modes,
selection, hyperlinks, and a 10,000-line scrollback. The canvas receives immutable terminal
snapshots; it does not interpret PTY bytes or own a second copy of terminal state.

Node body dimensions determine terminal rows and columns using a fixed 8-by-16 world-unit cell.
Resizing a node updates both the emulator and native PTY. Rendering scales cells with the canvas
camera, skips wide-character spacer cells, appends combining marks to their base glyph, applies
ANSI foreground/background/style attributes, draws the emulator cursor and selection, and
underlines OSC 8 hyperlinks.

## Input, focus, and clipboard

Clicking an agent node body focuses its terminal. While focused, keyboard and IME commit events are
encoded for the PTY, and canvas pan/zoom/edit shortcuts do not consume them. `Cmd/Ctrl+Shift+Esc`
releases terminal focus without taking the ordinary `Escape` key away from full-screen programs.
Clicking a header or the canvas background also releases focus.

The input mapper supports text, control characters, navigation, editing keys, function keys, and
application-cursor mode. Mouse dragging in the terminal body creates an emulator selection. The
mouse wheel scrolls terminal history unless the child has enabled mouse reporting. Platform-copy
copies the selection and platform-paste sends clipboard text, using bracketed paste when requested
by the terminal mode. OSC 52 copy requests are forwarded to the system clipboard; OSC 52 paste is
disabled by the emulator's default security policy.

## Known text limitations

Unicode width and combining characters follow `alacritty_terminal`. Iced performs glyph shaping,
but this implementation does not apply a terminal-specific bidi reordering pass; bidirectional
control sequences therefore depend on the selected system font and Iced text backend. IME commit
text is accepted, and pre-edit text is displayed when Iced delivers composition events to the
canvas; candidate-window positioning is controlled by Iced and the operating system.

## Verification

Targeted tests cover ANSI style and alternate-screen parsing, Unicode/wide/combining cells,
scrollback, selection, resize, bracketed paste, application-cursor keys, control-key mapping, and
canvas shortcut suppression while a terminal is focused. Existing PTY tests continue to prove
native input, output, resize, termination, and cancellation behavior.
