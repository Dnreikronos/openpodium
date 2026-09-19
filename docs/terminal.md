# Interactive terminal node contract

Status: Issue #8 implementation contract  
Updated: 2026-09-18

## Session lifecycle

Terminal sessions are ephemeral application state keyed by agent node ID. Creating an agent node
starts its selected program immediately in the workspace working directory. Restored nodes remain
offline until the user starts them, so reopening OpenPodium never launches commands unexpectedly.
Removing a node, switching workspaces, stopping a terminal, or closing the application cancels its
local process through the runtime boundary.

Shell nodes launch the user's login shell on Unix and `COMSPEC` on Windows. Codex and Claude nodes
launch the `codex` and `claude` executables. Every child receives `TERM=xterm-256color`,
`COLORTERM=truecolor`, and `TERM_PROGRAM=OpenPodium`. Custom commands and OpenCode require durable
launch metadata that is not present in the current domain model and remain follow-up work.

The node header reports `offline`, `starting`, `running`, `exited`, `stopped`, or `failed`. Runtime
handles and emulator state are never serialized.

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
