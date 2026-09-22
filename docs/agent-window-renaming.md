# Rename an agent window

Select an agent window and use the pencil in its header, or the Rename action
beside its terminal controls. Open a focused name field in a compact dialog.
Enter saves; Cancel, Escape, and the outside backdrop discard the draft.

Persist the agent's name, not the terminal's process-generated title. Existing
terminal context such as the current directory remains a suffix. Reject empty
or overlong names with inline feedback. Saving must not restart the process,
change the agent ID, or lose conversation history. Other cards for the same
agent show the same updated name.

The pencil's rendered bounds and hit target must agree at every zoom. Header
dragging outside that control, resize, terminal input, and connection mode keep
their existing behavior. Verify persistence, validation, cancel, runtime
identity, and zoom-aware hit testing with targeted tests.
