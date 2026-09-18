use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io;
use std::path::PathBuf;

use crate::domain::{DomainError, WorkspaceId};

#[derive(Debug)]
#[non_exhaustive]
pub enum PersistenceError {
    Database {
        operation: &'static str,
        source: rusqlite::Error,
    },
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    UnsupportedSchemaVersion {
        found: u32,
        supported: u32,
    },
    Migration {
        from: u32,
        to: u32,
        backup_path: Option<PathBuf>,
        source: Box<Self>,
    },
    IntegrityCheckFailed {
        detail: String,
    },
    Configuration {
        detail: String,
    },
    UnsupportedRecordVersion {
        record_type: &'static str,
        sequence: u64,
        found: u32,
        supported: u32,
    },
    ChecksumMismatch {
        record_type: &'static str,
        sequence: u64,
    },
    Serialization {
        record_type: &'static str,
        source: serde_json::Error,
    },
    Deserialization {
        record_type: &'static str,
        sequence: u64,
        source: serde_json::Error,
    },
    InvalidRecord {
        record_type: &'static str,
        sequence: u64,
        detail: String,
    },
    Domain(DomainError),
    UnknownWorkspace {
        workspace_id: WorkspaceId,
    },
    NoValidSnapshot {
        workspace_id: WorkspaceId,
        snapshots_checked: usize,
    },
}

impl PersistenceError {
    pub(crate) fn database(operation: &'static str, source: rusqlite::Error) -> Self {
        Self::Database { operation, source }
    }

    pub(crate) fn migration(
        from: u32,
        to: u32,
        backup_path: Option<PathBuf>,
        source: Self,
    ) -> Self {
        Self::Migration {
            from,
            to,
            backup_path,
            source: Box::new(source),
        }
    }

    pub(crate) fn invalid_record(
        record_type: &'static str,
        sequence: u64,
        detail: impl Into<String>,
    ) -> Self {
        Self::InvalidRecord {
            record_type,
            sequence,
            detail: detail.into(),
        }
    }
}

impl Display for PersistenceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database { operation, source } => {
                write!(formatter, "SQLite {operation} failed: {source}")
            }
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} {}: {source}",
                path.display()
            ),
            Self::UnsupportedSchemaVersion { found, supported } => write!(
                formatter,
                "database schema version {found} is newer than supported version {supported}; the database was not modified"
            ),
            Self::Migration {
                from,
                to,
                backup_path,
                source,
            } => {
                write!(
                    formatter,
                    "database migration from version {from} to {to} failed"
                )?;
                if let Some(path) = backup_path {
                    write!(formatter, "; backup: {}", path.display())?;
                }
                write!(formatter, ": {source}")
            }
            Self::IntegrityCheckFailed { detail } => {
                write!(formatter, "SQLite integrity check failed: {detail}")
            }
            Self::Configuration { detail } => {
                write!(formatter, "SQLite configuration failed: {detail}")
            }
            Self::UnsupportedRecordVersion {
                record_type,
                sequence,
                found,
                supported,
            } => write!(
                formatter,
                "{record_type} {sequence} uses format version {found}, but this build supports version {supported}"
            ),
            Self::ChecksumMismatch {
                record_type,
                sequence,
            } => write!(
                formatter,
                "{record_type} {sequence} failed its BLAKE3 checksum"
            ),
            Self::Serialization {
                record_type,
                source,
            } => write!(formatter, "could not serialize {record_type}: {source}"),
            Self::Deserialization {
                record_type,
                sequence,
                source,
            } => write!(
                formatter,
                "could not deserialize {record_type} {sequence}: {source}"
            ),
            Self::InvalidRecord {
                record_type,
                sequence,
                detail,
            } => write!(formatter, "invalid {record_type} {sequence}: {detail}"),
            Self::Domain(source) => write!(formatter, "domain command failed: {source}"),
            Self::UnknownWorkspace { workspace_id } => {
                write!(formatter, "workspace {workspace_id} is not registered")
            }
            Self::NoValidSnapshot {
                workspace_id,
                snapshots_checked,
            } => write!(
                formatter,
                "workspace {workspace_id} has journal data but none of its {snapshots_checked} snapshots are valid"
            ),
        }
    }
}

impl Error for PersistenceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database { source, .. } => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::Migration { source, .. } => Some(source),
            Self::Serialization { source, .. } => Some(source),
            Self::Deserialization { source, .. } => Some(source),
            Self::Domain(source) => Some(source),
            Self::UnsupportedSchemaVersion { .. }
            | Self::IntegrityCheckFailed { .. }
            | Self::Configuration { .. }
            | Self::UnsupportedRecordVersion { .. }
            | Self::ChecksumMismatch { .. }
            | Self::InvalidRecord { .. }
            | Self::UnknownWorkspace { .. }
            | Self::NoValidSnapshot { .. } => None,
        }
    }
}

impl From<DomainError> for PersistenceError {
    fn from(source: DomainError) -> Self {
        Self::Domain(source)
    }
}
