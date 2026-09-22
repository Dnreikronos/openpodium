# Agent window deletion

Deleting the last canvas window for an agent removes that agent from the active
workspace registry. The sidebar count and runtime registrations must immediately
reflect the removal. Another window for the same agent, including one on another
floor, keeps the agent active.

Preserve past conversations and task references as history. Move the removed
agent's historical record out of the active registry; do not erase the journal,
project files, notes, or shared tasks. Undo restores the window and its agent;
redo removes them again. Historical agent IDs must never be reused.

Persist the separation in snapshots. On loading an older workspace, identify
agents previously placed on the canvas from its journal and archive only those
with no remaining window. Do not remove agents created without a canvas window.

Verify deletion, shared windows, undo/redo, snapshot and journal recovery, legacy
cleanup, stable IDs, and removal from runtime registrations with targeted tests.
