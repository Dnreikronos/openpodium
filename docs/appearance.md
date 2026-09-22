# Appearance

Replace the sidebar's Display controls (language, text scale, contrast and motion)
with one Theme selector: System, Light, Dark. Default to System and persist only
the selected mode, not a copy of the desktop's current appearance. Read the
desktop appearance on launch and react to its changes while the app is open.
Explicit Light and Dark choices ignore desktop changes.

Apply the resolved palette to the sidebar, canvas, cards, dialogs and terminal
defaults. Preserve explicit terminal colors. Changing appearance must invalidate
cached canvas drawing without modifying node geometry or restarting processes.
Keep existing OS accessibility support and saved preferences internally; remove
the obsolete sidebar buttons and their unused UI handlers.

Verify preference round trips and invalid stored values, explicit overrides,
desktop changes, canvas invalidation, and readable terminal defaults in both
themes. Inspect the running UI in an isolated profile without interrupting the
user's terminals.

Legacy terminal snapshots stored default colors as explicit RGB. On restoration,
interpret their old white background and dark foreground (including dim/inverse
variants) as theme defaults, including scrollback. Old snapshots cannot distinguish
an intentionally identical RGB color from a default; this compatibility conversion
is limited to unmarked legacy snapshots. New snapshots mark semantic color support
and preserve explicit RGB colors exactly. Raw terminal output is not migrated.

Terminal color queries (OSC 10/11/12 and indexed colors) must report the colors
actually rendered. Derive foreground, background, and cursor defaults from the
resolved app palette for each incoming output event, without storing a second
theme preference in terminal sessions. Respect terminal palette overrides and
leave indexed ANSI colors unchanged. Test explicit Light/Dark, System changes,
color overrides/reset, and Codex's live input area in an isolated workspace.
Programs that cache colors at startup may need to be restarted after a theme change.
