# Local process runtime contract

Status: Issue #7 implementation contract
Updated: 2026-09-18

## Boundary

The runtime module owns local pseudo-terminals, child processes, raw input and
output, resize requests, cancellation, and process termination. It exposes
OpenPodium-owned request, event, size, and error types; `portable-pty` types and
live operating-system handles do not cross the module boundary and are never
part of serialized domain state.

The runtime publishes observations but does not mutate a workspace or choose an
agent lifecycle transition. The application layer will translate runtime
termination into domain commands. Terminal parsing, scrollback, rendering, and
keyboard encoding remain part of issue #8.

## Process lifecycle

Launching uses an argument vector rather than a shell command string. A launch
specification contains the executable, arguments, explicit working directory,
environment overrides, initial terminal size, and bounded output capacity.
Inherited environment variables are preserved unless an override replaces
one.

`spawn` returns only after the PTY, reader, writer, and child have all been
created. A failure before that point is returned once from `spawn`; no partially
started process handle escapes. A successful handle publishes exactly one
terminal event:

- `Exited` includes the portable exit code, optional signal, and success flag.
- `Cancelled` reports that OpenPodium requested termination.
- `Failed` reports a runtime failure while observing the child's exit or
  draining its PTY output.

Dropping a live handle has the same cleanup semantics as cancellation. Killing
an already-finished process is avoided, and cancellation is idempotent.

## I/O and backpressure

PTY input and output are opaque byte sequences. Input writes are serialized and
write directly to the PTY, so operating-system flow control applies without an
unbounded userspace input queue. Closing input explicitly drops the PTY writer
and delivers end-of-file to the child. Output is read in fixed-size chunks and
sent through a bounded Tokio channel. When the consumer falls behind, the
reader blocks at the configured bound and the PTY applies backpressure; output
is not silently dropped.

Output and termination use separate channels. A full output queue therefore
cannot hide or duplicate process termination. Cancellation closes the output
receiver before killing the child, which releases a reader blocked on a full
queue. The child waiter owns the only termination sender, making the terminal
event single-delivery by construction.

Resize calls update the native PTY size through the master handle. Rows and
columns must be non-zero; pixel dimensions may be zero when unknown.

## Verification

Targeted tests use temporary working directories and real native PTYs. They
drive an interactive platform shell, verify raw input/output and working
directory selection, observe a child-side resize, distinguish normal exit from
cancellation, reject an invalid executable at startup, exercise bounded output,
and confirm that dropping or cancelling a handle allows its child and worker
threads to terminate.
