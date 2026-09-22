# Reopening stopped terminals

Closing OpenPodium continues to stop its terminal processes. Reopening restores
saved output, not a running shell. Do not auto-launch programs, replay typed
commands, or imply that old shell variables and jobs survived.

Selected offline windows expose a Restart / Resume action. Typing or pasting into
one opens a compact recovery dialog instead of silently dropping input. Scrolling
and copying saved output remain available. Restart opens the original program in
the node's configured directory; shell cards also offer Codex and Claude resume
pickers for conversations originally launched inside Bash/Zsh. Native Codex and
Claude cards offer their corresponding picker. Do not use “last session” because
several cards can share a directory. Other/custom programs retain a restart action.

Keep the cached screen on launch failure. Close the dialog and focus the card on
successful launch. Test command arguments, working directory, no automatic
launch, stopped-input routing, and the live Bash/Codex flows in an isolated profile.

Bare printable shortcuts such as `0`, `+`, and `-` are text while a terminal or
portal has focus. They may control the canvas only when embedded content does not
have focus. Explicit modified shortcuts and the canvas toolbar remain available.
