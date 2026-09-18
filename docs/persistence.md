# OpenPodium persistence contract

Status: Issue #3 implementation contract  
Updated: 2026-09-18

## Boundary

The persistence module owns SQLite schema migration, the append-only domain
event journal, materialized workspace snapshots, integrity checks, and
recovery. It translates between versioned storage records and the
dependency-free domain model; database and serialization types do not cross
into `domain`.

## Atomic writes

`Journal::execute` is the only write path for a domain command. It clones the
workspace, validates and applies the command to the clone, and writes both the
resulting timeline event and a snapshot of the resulting aggregate in one
SQLite transaction. The caller's workspace is replaced only after that
transaction commits.

Consequently, a validation failure changes neither memory nor disk, a database
failure leaves the caller's workspace unchanged, and forced termination cannot
leave an event without its corresponding snapshot.

## Recovery

Events and snapshots have independent format versions and BLAKE3 checksums.
Recovery verifies SQLite's structural integrity, then tries snapshots newest
first. An invalid snapshot is skipped without being deleted. Once a valid
snapshot is found, later journal entries are verified and replayed in sequence
through the domain model. A corrupt journal entry or invalid replay stops
recovery with an actionable error rather than returning partial state.

Every successful command currently creates a snapshot. This intentionally
favours simple and deterministic crash recovery; snapshot compaction and a
lower snapshot cadence can be added when real workspace sizes justify them.

## Schema migration and backups

SQLite `PRAGMA user_version` is the schema version source of truth. Opening a
database whose version is newer than this build supports fails before enabling
WAL or executing any write. Opening an older, non-empty database creates a
SQLite-consistent sibling backup before migrations run. Each migration and its
`user_version` update commit together, and errors report the source and target
versions plus the backup path when one was created.

New and migrated databases use WAL mode, foreign keys, and a busy timeout.
Tests use temporary on-disk databases so close/reopen recovery, WAL behaviour,
rollback after an interrupted write, corrupt payload handling, backups, and
unknown schema rejection exercise the same storage path as the application.
