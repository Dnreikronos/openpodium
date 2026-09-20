# Portable templates and workspace archives

OpenPodium has two SQLite-independent JSON formats:

- `openpodium-template` stores a selected canvas fragment, the groups and
  internal connections in that fragment, and the roles, agents, tasks, and
  handoffs reached through node references.
- `openpodium-workspace-archive` stores the portable workspace allowlist:
  display settings, roles, agent configuration, the full canvas graph, and
  portable coordination data.

Both formats currently use version `1`, reject unknown fields, enforce bounded
document and collection sizes, and accept the version-0 migration shape used
by early fixtures. Entity references use document-local symbolic IDs such as
`agent-12`; importing allocates fresh domain IDs.

Template coordinates are relative to the minimum selected-node position. The
destination origin is supplied when the template is instantiated. Notes,
file trees, artifacts, and diffs contain only normalized project-relative
paths and note titles. File bytes, absolute paths, attachments, terminal
handles, floors, ownership tokens, environment profiles, and runtime state
are excluded.

Built-in agent programs remain portable. Custom programs are represented by a
symbolic launcher reference and must be mapped to a destination command
preset during the import preview. The archive exporter scans free-form fields
for common secret patterns and blocks export with field-level warnings.

Workspace-manager imports decode and migrate the document, build an
`ImportPreview`, revalidate every referenced path against the selected floor
with the symlink-aware project-path resolver, and submit the resulting domain
commands to one journal batch. The journal writes all events and snapshots in
one SQLite transaction and updates in-memory state only after commit. v1 never
writes project files.
