use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, TransactionBehavior, params};

use crate::domain::{
    DomainCommand, TimelineEvent, TimelineEventId, Timestamp, Workspace, WorkspaceId,
};

use super::PersistenceError;
use super::codec::{
    EVENT_FORMAT_VERSION, SNAPSHOT_FORMAT_VERSION, checksum, decode_event, decode_workspace,
    encode_event, encode_workspace,
};

const SCHEMA_VERSION: u32 = 1;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const MIGRATION_0_TO_1: &str = "
CREATE TABLE journal_events (
    sequence       INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_id   TEXT NOT NULL,
    occurred_at    TEXT NOT NULL,
    format_version INTEGER NOT NULL CHECK (format_version > 0),
    payload        BLOB NOT NULL,
    checksum       BLOB NOT NULL CHECK (length(checksum) = 32)
) STRICT;

CREATE INDEX journal_events_workspace_sequence
    ON journal_events (workspace_id, sequence);

CREATE TABLE workspace_snapshots (
    event_sequence INTEGER PRIMARY KEY
        REFERENCES journal_events(sequence) ON DELETE CASCADE,
    workspace_id   TEXT NOT NULL,
    format_version INTEGER NOT NULL CHECK (format_version > 0),
    payload        BLOB NOT NULL,
    checksum       BLOB NOT NULL CHECK (length(checksum) = 32)
) STRICT;

CREATE INDEX workspace_snapshots_workspace_sequence
    ON workspace_snapshots (workspace_id, event_sequence DESC);
";

pub struct Journal {
    connection: Connection,
    path: PathBuf,
}

impl Journal {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PersistenceError> {
        let path = path.as_ref().to_owned();
        let mut connection =
            Connection::open(&path).map_err(|source| PersistenceError::database("open", source))?;

        let found_version = schema_version(&connection)?;
        if found_version > SCHEMA_VERSION {
            return Err(PersistenceError::UnsupportedSchemaVersion {
                found: found_version,
                supported: SCHEMA_VERSION,
            });
        }

        if has_user_schema(&connection)? {
            verify_integrity(&connection)?;
        }
        if found_version < SCHEMA_VERSION {
            migrate(&mut connection, &path, found_version)?;
        }

        configure(&connection)?;

        Ok(Self { connection, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn execute(
        &mut self,
        workspace: &mut Workspace,
        command: DomainCommand,
        occurred_at: Timestamp,
    ) -> Result<TimelineEvent, PersistenceError> {
        let mut candidate = workspace.clone();
        let event = candidate.execute(command)?;
        let event_payload = encode_event(&event)?;
        let snapshot_payload = encode_workspace(&candidate)?;
        let workspace_key = workspace.id().get().to_string();
        let occurred_at_value = occurred_at.as_unix_millis();
        let occurred_at_key = occurred_at_value.to_string();
        let event_version = EVENT_FORMAT_VERSION.to_le_bytes();
        let timestamp_bytes = occurred_at_value.to_le_bytes();
        let event_checksum = checksum(&[
            &event_version,
            workspace_key.as_bytes(),
            &timestamp_bytes,
            &event_payload,
        ]);

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|source| PersistenceError::database("begin write transaction", source))?;

        transaction
            .execute(
                "INSERT INTO journal_events (
                    workspace_id, occurred_at, format_version, payload, checksum
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    workspace_key,
                    occurred_at_key,
                    i64::from(EVENT_FORMAT_VERSION),
                    event_payload,
                    &event_checksum[..],
                ],
            )
            .map_err(|source| PersistenceError::database("insert domain event", source))?;

        let sequence = positive_sequence(transaction.last_insert_rowid(), "domain event")?;
        let snapshot_version = SNAPSHOT_FORMAT_VERSION.to_le_bytes();
        let sequence_bytes = sequence.to_le_bytes();
        let snapshot_checksum = checksum(&[
            &snapshot_version,
            workspace_key.as_bytes(),
            &sequence_bytes,
            &snapshot_payload,
        ]);

        transaction
            .execute(
                "INSERT INTO workspace_snapshots (
                    event_sequence, workspace_id, format_version, payload, checksum
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    i64::try_from(sequence).expect("SQLite row IDs fit in i64"),
                    workspace_key,
                    i64::from(SNAPSHOT_FORMAT_VERSION),
                    snapshot_payload,
                    &snapshot_checksum[..],
                ],
            )
            .map_err(|source| PersistenceError::database("insert workspace snapshot", source))?;

        transaction
            .commit()
            .map_err(|source| PersistenceError::database("commit domain event", source))?;

        *workspace = candidate;
        Ok(TimelineEvent::new(
            TimelineEventId::new(sequence),
            workspace.id(),
            occurred_at,
            event,
        ))
    }

    pub fn recover(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<Workspace>, PersistenceError> {
        verify_integrity(&self.connection)?;

        let workspace_key = workspace_id.get().to_string();
        let snapshots = load_snapshots(&self.connection, &workspace_key)?;
        let snapshots_checked = snapshots.len();
        let mut recovered = None;

        for snapshot in snapshots {
            if let Ok(workspace) = snapshot.decode(&workspace_key, workspace_id) {
                recovered = Some((snapshot.sequence, workspace));
                break;
            }
        }

        let Some((snapshot_sequence, mut workspace)) = recovered else {
            let event_count: i64 = self
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM journal_events WHERE workspace_id = ?1",
                    [&workspace_key],
                    |row| row.get(0),
                )
                .map_err(|source| PersistenceError::database("count domain events", source))?;

            if event_count == 0 {
                return Ok(None);
            }
            return Err(PersistenceError::NoValidSnapshot {
                workspace_id,
                snapshots_checked,
            });
        };

        for event in load_events_after(&self.connection, &workspace_key, snapshot_sequence)? {
            let timeline_event = event.decode(&workspace_key, workspace_id)?;
            workspace.replay(&timeline_event).map_err(|error| {
                PersistenceError::invalid_record(
                    "domain event",
                    timeline_event.id().get(),
                    error.to_string(),
                )
            })?;
        }

        Ok(Some(workspace))
    }

    #[cfg(test)]
    pub(super) fn connection(&self) -> &Connection {
        &self.connection
    }
}

#[derive(Debug)]
struct SnapshotRecord {
    sequence: u64,
    format_version: u32,
    payload: Vec<u8>,
    checksum: Vec<u8>,
}

impl SnapshotRecord {
    fn decode(
        &self,
        workspace_key: &str,
        workspace_id: WorkspaceId,
    ) -> Result<Workspace, PersistenceError> {
        if self.format_version != SNAPSHOT_FORMAT_VERSION {
            return Err(PersistenceError::UnsupportedRecordVersion {
                record_type: "workspace snapshot",
                sequence: self.sequence,
                found: self.format_version,
                supported: SNAPSHOT_FORMAT_VERSION,
            });
        }

        let version = self.format_version.to_le_bytes();
        let sequence = self.sequence.to_le_bytes();
        let expected = checksum(&[&version, workspace_key.as_bytes(), &sequence, &self.payload]);
        if self.checksum.as_slice() != expected {
            return Err(PersistenceError::ChecksumMismatch {
                record_type: "workspace snapshot",
                sequence: self.sequence,
            });
        }

        let workspace = decode_workspace(&self.payload, self.sequence)?;
        if workspace.id() != workspace_id {
            return Err(PersistenceError::invalid_record(
                "workspace snapshot",
                self.sequence,
                format!(
                    "payload belongs to workspace {}, not workspace {workspace_id}",
                    workspace.id()
                ),
            ));
        }
        Ok(workspace)
    }
}

#[derive(Debug)]
struct EventRecord {
    sequence: u64,
    occurred_at: String,
    format_version: u32,
    payload: Vec<u8>,
    checksum: Vec<u8>,
}

impl EventRecord {
    fn decode(
        self,
        workspace_key: &str,
        workspace_id: WorkspaceId,
    ) -> Result<TimelineEvent, PersistenceError> {
        if self.format_version != EVENT_FORMAT_VERSION {
            return Err(PersistenceError::UnsupportedRecordVersion {
                record_type: "domain event",
                sequence: self.sequence,
                found: self.format_version,
                supported: EVENT_FORMAT_VERSION,
            });
        }

        let timestamp = self.occurred_at.parse::<u64>().map_err(|error| {
            PersistenceError::invalid_record(
                "domain event",
                self.sequence,
                format!("invalid occurred_at value: {error}"),
            )
        })?;
        let version = self.format_version.to_le_bytes();
        let timestamp_bytes = timestamp.to_le_bytes();
        let expected = checksum(&[
            &version,
            workspace_key.as_bytes(),
            &timestamp_bytes,
            &self.payload,
        ]);
        if self.checksum.as_slice() != expected {
            return Err(PersistenceError::ChecksumMismatch {
                record_type: "domain event",
                sequence: self.sequence,
            });
        }

        Ok(TimelineEvent::new(
            TimelineEventId::new(self.sequence),
            workspace_id,
            Timestamp::from_unix_millis(timestamp),
            decode_event(&self.payload, self.sequence)?,
        ))
    }
}

fn load_snapshots(
    connection: &Connection,
    workspace_key: &str,
) -> Result<Vec<SnapshotRecord>, PersistenceError> {
    let mut statement = connection
        .prepare(
            "SELECT event_sequence, format_version, payload, checksum
             FROM workspace_snapshots
             WHERE workspace_id = ?1
             ORDER BY event_sequence DESC",
        )
        .map_err(|source| PersistenceError::database("prepare snapshot recovery", source))?;
    let rows = statement
        .query_map([workspace_key], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })
        .map_err(|source| PersistenceError::database("query workspace snapshots", source))?;

    let mut snapshots = Vec::new();
    for row in rows {
        let (sequence, format_version, payload, checksum) =
            row.map_err(|source| PersistenceError::database("read workspace snapshot", source))?;
        snapshots.push(SnapshotRecord {
            sequence: positive_sequence(sequence, "workspace snapshot")?,
            format_version: record_version(format_version, "workspace snapshot", sequence)?,
            payload,
            checksum,
        });
    }
    Ok(snapshots)
}

fn load_events_after(
    connection: &Connection,
    workspace_key: &str,
    snapshot_sequence: u64,
) -> Result<Vec<EventRecord>, PersistenceError> {
    let mut statement = connection
        .prepare(
            "SELECT sequence, occurred_at, format_version, payload, checksum
             FROM journal_events
             WHERE workspace_id = ?1 AND sequence > ?2
             ORDER BY sequence",
        )
        .map_err(|source| PersistenceError::database("prepare event replay", source))?;
    let rows = statement
        .query_map(
            params![
                workspace_key,
                i64::try_from(snapshot_sequence).expect("SQLite row IDs fit in i64")
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                ))
            },
        )
        .map_err(|source| PersistenceError::database("query domain events", source))?;

    let mut events = Vec::new();
    for row in rows {
        let (sequence, occurred_at, format_version, payload, checksum) =
            row.map_err(|source| PersistenceError::database("read domain event", source))?;
        events.push(EventRecord {
            sequence: positive_sequence(sequence, "domain event")?,
            occurred_at,
            format_version: record_version(format_version, "domain event", sequence)?,
            payload,
            checksum,
        });
    }
    Ok(events)
}

fn schema_version(connection: &Connection) -> Result<u32, PersistenceError> {
    let version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(|source| PersistenceError::database("read schema version", source))?;
    u32::try_from(version).map_err(|_| PersistenceError::Configuration {
        detail: format!("invalid negative schema version {version}"),
    })
}

fn has_user_schema(connection: &Connection) -> Result<bool, PersistenceError> {
    connection
        .query_row(
            "SELECT EXISTS (
                SELECT 1 FROM sqlite_schema
                WHERE name NOT LIKE 'sqlite_%'
            )",
            [],
            |row| row.get(0),
        )
        .map_err(|source| PersistenceError::database("inspect existing schema", source))
}

fn migrate(connection: &mut Connection, path: &Path, from: u32) -> Result<(), PersistenceError> {
    let to = from + 1;
    let backup_path = if has_user_schema(connection)? {
        let backup_path = next_backup_path(path, from, to)?;
        connection
            .backup(rusqlite::MAIN_DB, &backup_path, None)
            .map_err(|source| {
                PersistenceError::migration(
                    from,
                    to,
                    Some(backup_path.clone()),
                    PersistenceError::database("create pre-migration backup", source),
                )
            })?;
        Some(backup_path)
    } else {
        None
    };

    let result = (|| {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|source| PersistenceError::database("begin migration", source))?;

        match from {
            0 => transaction
                .execute_batch(MIGRATION_0_TO_1)
                .map_err(|source| PersistenceError::database("apply schema version 1", source))?,
            _ => {
                return Err(PersistenceError::Configuration {
                    detail: format!("no migration exists from schema version {from}"),
                });
            }
        }

        transaction
            .pragma_update(None, "user_version", i64::from(to))
            .map_err(|source| PersistenceError::database("update schema version", source))?;
        transaction
            .commit()
            .map_err(|source| PersistenceError::database("commit migration", source))
    })();

    result.map_err(|source| PersistenceError::migration(from, to, backup_path, source))
}

fn next_backup_path(path: &Path, from: u32, to: u32) -> Result<PathBuf, PersistenceError> {
    let mut file_name = path
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("openpodium.sqlite"));
    file_name.push(format!(".backup-v{from}-before-v{to}"));
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let base = parent.join(file_name);
    let mut candidate = base.clone();
    let mut suffix = 1_u32;

    while candidate
        .try_exists()
        .map_err(|source| PersistenceError::Io {
            operation: "check backup path",
            path: candidate.clone(),
            source,
        })?
    {
        let mut name = base.as_os_str().to_os_string();
        name.push(format!(".{suffix}"));
        candidate = PathBuf::from(name);
        suffix += 1;
    }

    Ok(candidate)
}

fn configure(connection: &Connection) -> Result<(), PersistenceError> {
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|source| PersistenceError::database("set busy timeout", source))?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|source| PersistenceError::database("enable foreign keys", source))?;
    let foreign_keys = connection
        .pragma_query_value(None, "foreign_keys", |row| row.get::<_, i64>(0))
        .map_err(|source| PersistenceError::database("verify foreign keys", source))?;
    if foreign_keys != 1 {
        return Err(PersistenceError::Configuration {
            detail: "foreign key enforcement could not be enabled".to_owned(),
        });
    }

    let journal_mode = connection
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
        .map_err(|source| PersistenceError::database("enable WAL mode", source))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(PersistenceError::Configuration {
            detail: format!("requested WAL mode, but SQLite selected {journal_mode}"),
        });
    }

    Ok(())
}

fn verify_integrity(connection: &Connection) -> Result<(), PersistenceError> {
    let mut results = Vec::new();
    connection
        .pragma_query(None, "quick_check", |row| {
            results.push(row.get::<_, String>(0)?);
            Ok(())
        })
        .map_err(|source| PersistenceError::database("run integrity check", source))?;

    if results.as_slice() == ["ok"] {
        Ok(())
    } else {
        Err(PersistenceError::IntegrityCheckFailed {
            detail: if results.is_empty() {
                "SQLite returned no result".to_owned()
            } else {
                results.join("; ")
            },
        })
    }
}

fn positive_sequence(value: i64, record_type: &'static str) -> Result<u64, PersistenceError> {
    u64::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            PersistenceError::invalid_record(
                record_type,
                value.unsigned_abs(),
                format!("invalid sequence {value}"),
            )
        })
}

fn record_version(
    value: i64,
    record_type: &'static str,
    sequence: i64,
) -> Result<u32, PersistenceError> {
    u32::try_from(value).map_err(|_| {
        PersistenceError::invalid_record(
            record_type,
            sequence.unsigned_abs(),
            format!("invalid format version {value}"),
        )
    })
}
