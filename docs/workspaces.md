# Workspace management contract

Status: Issue #4 implementation contract
Updated: 2026-09-18

## Boundary

Workspace settings are durable domain state. A workspace has a stable identifier,
name, optional icon, canonical local working directory, and optional instructions.
The application layer translates between operating-system paths and the domain's
validated path text; filesystem access does not cross into `domain`.

`WorkspaceManager` owns the set of loaded workspace aggregates and only changes
the active workspace identifier when switching. Runtime processes will be keyed
by workspace identifier, so changing the visible workspace cannot implicitly
stop, replace, or drop an inactive workspace's agents.

## Directory safety

Creating a workspace or changing its directory performs read-only validation:

- the path must exist and identify a directory;
- directory entries must be readable;
- the canonical path must be representable as Unicode for durable storage; and
- validation errors retain the path and underlying operating-system error.

Workspace operations never create, edit, or delete content inside the selected
project directory. OpenPodium's database lives in the platform application-data
directory, outside every project workspace.

## Persistence and restore

The SQLite schema records known workspace identifiers, recency, and the active
selection. Workspace settings remain part of the versioned domain event and
snapshot formats rather than being duplicated in an application table.

Creating a workspace writes its initial settings event and snapshot before it is
made active. Editing settings uses the same journal transaction as other domain
changes. Switching atomically updates the active identifier and recency without
changing either workspace aggregate.

At startup, the manager loads known workspaces in most-recently-used order,
recovers every aggregate through the journal, and restores the recorded active
identifier. A missing or corrupt referenced workspace is an actionable recovery
error rather than a silent fallback that could hide user state.

## Verification

Targeted tests cover invalid and inaccessible directory errors, metadata and
active-selection restoration after reopening SQLite, preservation of a running
agent while switching away and back, schema migration, and the absence of files
created inside project directories.
