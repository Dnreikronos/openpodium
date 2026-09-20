use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::{PortalServiceError, now_ms};
use crate::ipc::{MessageId, PortalActionReceipt, PortalActionState, PortalPolicyOutcome};

const APPROVAL_LOST_ON_RESTART: &str = "approval request did not survive a restart";

#[derive(Debug)]
pub(super) struct PortalActionJournal {
    connection: Connection,
}

pub(super) struct JournalIntent<'a> {
    pub workspace_id: u64,
    pub agent_id: u64,
    pub portal_id: u64,
    pub action_id: &'a MessageId,
    pub fingerprint: &'a str,
    pub state: &'a str,
    pub policy: &'a str,
    pub policy_detail: Option<String>,
    pub created_at_ms: u64,
}

impl PortalActionJournal {
    pub(super) fn open(path: impl AsRef<Path>) -> Result<Self, PortalServiceError> {
        let connection = Connection::open(path).map_err(journal_error)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = FULL;
                 CREATE TABLE IF NOT EXISTS portal_actions (
                     workspace_id   INTEGER NOT NULL,
                     action_id      TEXT NOT NULL,
                     agent_id       INTEGER NOT NULL,
                     portal_id      INTEGER NOT NULL,
                     fingerprint    TEXT NOT NULL,
                     state          TEXT NOT NULL,
                     policy         TEXT NOT NULL,
                     policy_detail  TEXT,
                     created_at_ms  INTEGER NOT NULL,
                     dispatched_at_ms INTEGER,
                     finished_at_ms INTEGER,
                     outcome        TEXT,
                     PRIMARY KEY (workspace_id, action_id)
                 ) STRICT;",
            )
            .map_err(journal_error)?;
        connection
            .execute(
                "UPDATE portal_actions
                 SET state = 'unknown',
                     outcome = COALESCE(outcome, 'execution outcome unknown after restart'),
                     finished_at_ms = NULL
                 WHERE state IN ('queued', 'dispatched')",
                [],
            )
            .map_err(journal_error)?;
        // Approval state lives in memory, so a surviving request can never be
        // granted. Fail it during recovery instead of leaving the client to
        // poll an approval that no longer exists.
        connection
            .execute(
                "UPDATE portal_actions
                 SET state = 'failed',
                     policy = 'denied',
                     policy_detail = ?1,
                     outcome = COALESCE(outcome, ?1),
                     finished_at_ms = ?2
                 WHERE state = 'awaiting_approval'",
                params![APPROVAL_LOST_ON_RESTART, now_ms()],
            )
            .map_err(journal_error)?;
        Ok(Self { connection })
    }

    pub(super) fn insert_intent(
        &self,
        intent: JournalIntent<'_>,
    ) -> Result<PortalActionReceipt, PortalServiceError> {
        if let Some(existing) = self.record(intent.workspace_id, intent.action_id)? {
            if existing.agent_id == intent.agent_id
                && existing.portal_id == intent.portal_id
                && existing.fingerprint == intent.fingerprint
            {
                return existing.into_receipt(true);
            }
            return Err(PortalServiceError::ActionConflict(
                intent.action_id.to_string(),
            ));
        }
        self.connection
            .execute(
                "INSERT INTO portal_actions (
                    workspace_id, action_id, agent_id, portal_id, fingerprint,
                    state, policy, policy_detail, created_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    intent.workspace_id,
                    intent.action_id.as_str(),
                    intent.agent_id,
                    intent.portal_id,
                    intent.fingerprint,
                    intent.state,
                    intent.policy,
                    intent.policy_detail,
                    intent.created_at_ms,
                ],
            )
            .map_err(journal_error)?;
        self.record(intent.workspace_id, intent.action_id)?
            .ok_or_else(|| PortalServiceError::MissingReceipt(intent.action_id.to_string()))?
            .into_receipt(false)
    }

    pub(super) fn mark_dispatched(
        &self,
        workspace_id: u64,
        action_id: &MessageId,
        dispatched_at_ms: u64,
    ) -> Result<(), PortalServiceError> {
        self.connection
            .execute(
                "UPDATE portal_actions
                 SET state = 'dispatched', dispatched_at_ms = ?3
                 WHERE workspace_id = ?1 AND action_id = ?2",
                params![workspace_id, action_id.as_str(), dispatched_at_ms],
            )
            .map(|_| ())
            .map_err(journal_error)
    }

    pub(super) fn mark_approved(
        &self,
        workspace_id: u64,
        action_id: &MessageId,
    ) -> Result<(), PortalServiceError> {
        self.connection
            .execute(
                "UPDATE portal_actions
                 SET state = 'queued', policy = 'allowed', policy_detail = NULL
                 WHERE workspace_id = ?1 AND action_id = ?2",
                params![workspace_id, action_id.as_str()],
            )
            .map(|_| ())
            .map_err(journal_error)
    }

    pub(super) fn mark_rejected(
        &self,
        workspace_id: u64,
        action_id: &MessageId,
        reason: &str,
        finished_at_ms: u64,
    ) -> Result<(), PortalServiceError> {
        self.connection
            .execute(
                "UPDATE portal_actions
                 SET state = 'failed', policy = 'denied', policy_detail = ?3,
                     outcome = ?3, finished_at_ms = ?4
                 WHERE workspace_id = ?1 AND action_id = ?2",
                params![workspace_id, action_id.as_str(), reason, finished_at_ms],
            )
            .map(|_| ())
            .map_err(journal_error)
    }

    pub(super) fn finish(
        &self,
        workspace_id: u64,
        action_id: &MessageId,
        state: &str,
        outcome: Option<String>,
        finished_at_ms: u64,
    ) -> Result<(), PortalServiceError> {
        self.connection
            .execute(
                "UPDATE portal_actions
                 SET state = ?3, outcome = ?4, finished_at_ms = ?5
                 WHERE workspace_id = ?1 AND action_id = ?2",
                params![
                    workspace_id,
                    action_id.as_str(),
                    state,
                    outcome,
                    finished_at_ms
                ],
            )
            .map(|_| ())
            .map_err(journal_error)
    }

    pub(super) fn receipt(
        &self,
        workspace_id: u64,
        action_id: &MessageId,
    ) -> Result<Option<PortalActionReceipt>, PortalServiceError> {
        self.record(workspace_id, action_id)?
            .map(|record| record.into_receipt(false))
            .transpose()
    }

    /// The agent that recorded the action, which stays available after the
    /// portal it ran against is gone.
    pub(super) fn sender(
        &self,
        workspace_id: u64,
        action_id: &MessageId,
    ) -> Result<Option<u64>, PortalServiceError> {
        Ok(self
            .record(workspace_id, action_id)?
            .map(|record| record.agent_id))
    }

    pub(super) fn duplicate_receipt(
        &self,
        workspace_id: u64,
        action_id: &MessageId,
        agent_id: u64,
        portal_id: u64,
        fingerprint: &str,
    ) -> Result<Option<PortalActionReceipt>, PortalServiceError> {
        let Some(record) = self.record(workspace_id, action_id)? else {
            return Ok(None);
        };
        if record.agent_id != agent_id
            || record.portal_id != portal_id
            || record.fingerprint != fingerprint
        {
            return Err(PortalServiceError::ActionConflict(action_id.to_string()));
        }
        record.into_receipt(true).map(Some)
    }

    fn record(
        &self,
        workspace_id: u64,
        action_id: &MessageId,
    ) -> Result<Option<JournalRecord>, PortalServiceError> {
        self.connection
            .query_row(
                "SELECT agent_id, portal_id, fingerprint, state, policy,
                        policy_detail, created_at_ms, dispatched_at_ms,
                        finished_at_ms, outcome
                 FROM portal_actions
                 WHERE workspace_id = ?1 AND action_id = ?2",
                params![workspace_id, action_id.as_str()],
                |row| {
                    Ok(JournalRecord {
                        action_id: action_id.clone(),
                        agent_id: row.get(0)?,
                        portal_id: row.get(1)?,
                        fingerprint: row.get(2)?,
                        state: row.get(3)?,
                        policy: row.get(4)?,
                        policy_detail: row.get(5)?,
                        created_at_ms: row.get(6)?,
                        dispatched_at_ms: row.get(7)?,
                        finished_at_ms: row.get(8)?,
                        outcome: row.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(journal_error)
    }
}

#[derive(Debug)]
struct JournalRecord {
    action_id: MessageId,
    agent_id: u64,
    portal_id: u64,
    fingerprint: String,
    state: String,
    policy: String,
    policy_detail: Option<String>,
    created_at_ms: u64,
    dispatched_at_ms: Option<u64>,
    finished_at_ms: Option<u64>,
    outcome: Option<String>,
}

impl JournalRecord {
    fn into_receipt(self, duplicate: bool) -> Result<PortalActionReceipt, PortalServiceError> {
        let policy = match self.policy.as_str() {
            "allowed" => PortalPolicyOutcome::Allowed,
            "approval_required" => {
                let detail = self.policy_detail.unwrap_or_default();
                let (approval_id, reason) = detail.split_once(':').ok_or_else(|| {
                    PortalServiceError::Journal("approval receipt is malformed".to_owned())
                })?;
                PortalPolicyOutcome::ApprovalRequired {
                    approval_id: approval_id.parse().map_err(|_| {
                        PortalServiceError::Journal("approval ID is malformed".to_owned())
                    })?,
                    reason: reason.to_owned(),
                }
            }
            "denied" => PortalPolicyOutcome::Denied {
                reason: self.policy_detail.unwrap_or_default(),
            },
            policy => {
                return Err(PortalServiceError::Journal(format!(
                    "unknown portal policy state {policy:?}"
                )));
            }
        };
        let state = match self.state.as_str() {
            "queued" => PortalActionState::Queued,
            "awaiting_approval" => PortalActionState::AwaitingApproval,
            "dispatched" => PortalActionState::Dispatched,
            "completed" => PortalActionState::Completed,
            "failed" => PortalActionState::Failed,
            "unknown" => PortalActionState::Unknown,
            state => {
                return Err(PortalServiceError::Journal(format!(
                    "unknown portal action state {state:?}"
                )));
            }
        };
        Ok(PortalActionReceipt {
            action_id: self.action_id,
            portal_id: self.portal_id,
            duplicate,
            state,
            policy,
            created_at_ms: self.created_at_ms,
            dispatched_at_ms: self.dispatched_at_ms,
            finished_at_ms: self.finished_at_ms,
            outcome: self.outcome,
        })
    }
}

fn journal_error(error: rusqlite::Error) -> PortalServiceError {
    PortalServiceError::Journal(error.to_string())
}
