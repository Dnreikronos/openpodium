use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io;
use std::path::PathBuf;

use crate::domain::{ValidationError, WorkspaceId};
use crate::persistence::{PersistenceError, PortableError};

#[derive(Debug)]
#[non_exhaustive]
pub enum WorkspaceError {
    Persistence(PersistenceError),
    Portable(PortableError),
    Validation(ValidationError),
    DirectoryAccess {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    NotDirectory {
        path: PathBuf,
    },
    NonUnicodeDirectory {
        path: PathBuf,
    },
    UnknownWorkspace {
        workspace_id: WorkspaceId,
    },
    MissingPersistedWorkspace {
        workspace_id: WorkspaceId,
    },
    InvalidImportDestination {
        floor: Option<u64>,
    },
    WorkspaceIdExhausted,
}

impl Display for WorkspaceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Persistence(source) => source.fmt(formatter),
            Self::Portable(source) => source.fmt(formatter),
            Self::Validation(source) => source.fmt(formatter),
            Self::DirectoryAccess {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "cannot {operation} workspace directory {}: {source}",
                path.display()
            ),
            Self::NotDirectory { path } => {
                write!(
                    formatter,
                    "workspace path {} is not a directory",
                    path.display()
                )
            }
            Self::NonUnicodeDirectory { path } => write!(
                formatter,
                "workspace directory {} cannot be stored because its path is not valid Unicode",
                path.display()
            ),
            Self::UnknownWorkspace { workspace_id } => {
                write!(formatter, "workspace {workspace_id} is not loaded")
            }
            Self::MissingPersistedWorkspace { workspace_id } => write!(
                formatter,
                "workspace {workspace_id} is registered but has no recoverable snapshot"
            ),
            Self::WorkspaceIdExhausted => {
                formatter.write_str("cannot allocate another workspace identifier")
            }
            Self::InvalidImportDestination { floor } => write!(
                formatter,
                "import destination floor {floor:?} is not the active destination"
            ),
        }
    }
}

impl Error for WorkspaceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Persistence(source) => Some(source),
            Self::Portable(source) => Some(source),
            Self::Validation(source) => Some(source),
            Self::DirectoryAccess { source, .. } => Some(source),
            Self::NotDirectory { .. }
            | Self::NonUnicodeDirectory { .. }
            | Self::UnknownWorkspace { .. }
            | Self::MissingPersistedWorkspace { .. }
            | Self::InvalidImportDestination { .. }
            | Self::WorkspaceIdExhausted => None,
        }
    }
}

impl From<PersistenceError> for WorkspaceError {
    fn from(source: PersistenceError) -> Self {
        Self::Persistence(source)
    }
}

impl From<PortableError> for WorkspaceError {
    fn from(source: PortableError) -> Self {
        Self::Portable(source)
    }
}

impl From<ValidationError> for WorkspaceError {
    fn from(source: ValidationError) -> Self {
        Self::Validation(source)
    }
}
