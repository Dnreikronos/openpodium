use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};

use super::{AcceptedMessage, MessageId, ProtocolCommand};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) struct MessageStore {
    connection: Connection,
}

#[derive(Debug)]
pub(super) struct StoredMessage {
    pub sender_agent_id: u64,
    pub recipient_agent_id: u64,
    pub command: ProtocolCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InsertResult {
    Inserted,
    Duplicate,
    Conflict,
    CapacityExhausted,
}

impl MessageStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let connection =
            Connection::open(path).map_err(|source| StoreError::database("open", source))?;
        connection
            .busy_timeout(BUSY_TIMEOUT)
            .map_err(|source| StoreError::database("configure busy timeout", source))?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = FULL;
                 CREATE TABLE IF NOT EXISTS ipc_messages (
                     sequence           INTEGER PRIMARY KEY AUTOINCREMENT,
                     workspace_id       TEXT NOT NULL,
                     message_id         TEXT NOT NULL,
                     sender_agent_id    TEXT NOT NULL,
                     recipient_agent_id TEXT NOT NULL,
                     command_json       TEXT NOT NULL,
                     processed          INTEGER NOT NULL DEFAULT 0 CHECK (processed IN (0, 1)),
                     UNIQUE (workspace_id, message_id)
                 ) STRICT;
                 CREATE INDEX IF NOT EXISTS ipc_messages_pending
                     ON ipc_messages (processed, sequence);",
            )
            .map_err(|source| StoreError::database("initialize", source))?;
        Ok(Self { connection })
    }

    pub fn message(
        &self,
        workspace_id: u64,
        message_id: &MessageId,
    ) -> Result<Option<StoredMessage>, StoreError> {
        self.connection
            .query_row(
                "SELECT sender_agent_id, recipient_agent_id, command_json
                 FROM ipc_messages
                 WHERE workspace_id = ?1 AND message_id = ?2",
                params![workspace_id.to_string(), message_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|source| StoreError::database("read message", source))?
            .map(|(sender, recipient, command)| {
                Ok(StoredMessage {
                    sender_agent_id: parse_id(&sender, "sender agent")?,
                    recipient_agent_id: parse_id(&recipient, "recipient agent")?,
                    command: serde_json::from_str(&command).map_err(StoreError::Decode)?,
                })
            })
            .transpose()
    }

    pub fn insert(
        &mut self,
        message: &AcceptedMessage,
        capacity: usize,
    ) -> Result<InsertResult, StoreError> {
        let message_id = message
            .command
            .message_id()
            .expect("accepted messages always have an ID");
        if let Some(existing) = self.message(message.workspace_id, message_id)? {
            return Ok(
                if existing.sender_agent_id == message.sender_agent_id
                    && existing.recipient_agent_id == message.recipient_agent_id
                    && existing.command == message.command
                {
                    InsertResult::Duplicate
                } else {
                    InsertResult::Conflict
                },
            );
        }

        let count = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM ipc_messages WHERE processed = 0",
                [],
                |row| row.get::<_, u64>(0),
            )
            .map_err(|source| StoreError::database("count messages", source))?;
        if count >= u64::try_from(capacity).expect("message capacity fits in u64") {
            return Ok(InsertResult::CapacityExhausted);
        }

        let command = serde_json::to_string(&message.command).map_err(StoreError::Encode)?;
        self.connection
            .execute(
                "INSERT INTO ipc_messages (
                    workspace_id, message_id, sender_agent_id,
                    recipient_agent_id, command_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    message.workspace_id.to_string(),
                    message_id.as_str(),
                    message.sender_agent_id.to_string(),
                    message.recipient_agent_id.to_string(),
                    command,
                ],
            )
            .map_err(|source| StoreError::database("insert message", source))?;
        Ok(InsertResult::Inserted)
    }

    pub fn next_pending(
        &self,
        leased: &BTreeSet<(u64, MessageId)>,
    ) -> Result<Option<AcceptedMessage>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT workspace_id, message_id, sender_agent_id,
                        recipient_agent_id, command_json
                 FROM ipc_messages
                 WHERE processed = 0
                 ORDER BY sequence",
            )
            .map_err(|source| StoreError::database("prepare pending messages", source))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|source| StoreError::database("query pending messages", source))?;

        for row in rows {
            let (workspace, message_id, sender, recipient, command) =
                row.map_err(|source| StoreError::database("read pending message", source))?;
            let workspace_id = parse_id(&workspace, "workspace")?;
            let message_id = MessageId::new(message_id).map_err(|error| {
                StoreError::InvalidData(format!("stored message ID is invalid: {error}"))
            })?;
            if leased.contains(&(workspace_id, message_id.clone())) {
                continue;
            }
            return Ok(Some(AcceptedMessage {
                workspace_id,
                sender_agent_id: parse_id(&sender, "sender agent")?,
                recipient_agent_id: parse_id(&recipient, "recipient agent")?,
                command: serde_json::from_str(&command).map_err(StoreError::Decode)?,
            }));
        }
        Ok(None)
    }

    pub fn mark_processed(
        &self,
        workspace_id: u64,
        message_id: &MessageId,
    ) -> Result<bool, StoreError> {
        self.connection
            .execute(
                "UPDATE ipc_messages
                 SET processed = 1
                 WHERE workspace_id = ?1 AND message_id = ?2 AND processed = 0",
                params![workspace_id.to_string(), message_id.as_str()],
            )
            .map(|updated| updated == 1)
            .map_err(|source| StoreError::database("mark message processed", source))
    }
}

fn parse_id(value: &str, field: &'static str) -> Result<u64, StoreError> {
    value
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| StoreError::InvalidData(format!("stored {field} ID is invalid")))
}

#[derive(Debug)]
pub(super) enum StoreError {
    Database {
        operation: &'static str,
        source: rusqlite::Error,
    },
    Encode(serde_json::Error),
    Decode(serde_json::Error),
    InvalidData(String),
}

impl StoreError {
    fn database(operation: &'static str, source: rusqlite::Error) -> Self {
        Self::Database { operation, source }
    }
}

impl Display for StoreError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database { operation, source } => {
                write!(
                    formatter,
                    "failed to {operation} durable IPC state: {source}"
                )
            }
            Self::Encode(source) => write!(formatter, "failed to encode IPC message: {source}"),
            Self::Decode(source) => write!(formatter, "failed to decode IPC message: {source}"),
            Self::InvalidData(message) => formatter.write_str(message),
        }
    }
}

impl Error for StoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database { source, .. } => Some(source),
            Self::Encode(source) | Self::Decode(source) => Some(source),
            Self::InvalidData(_) => None,
        }
    }
}
