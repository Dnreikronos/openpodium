use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::portal::{
    CapabilityStatus, PolicyDecision, PolicyRequest, PortalAction, PortalBackend,
    PortalCapabilities, PortalConfig, PortalObservation as CoreObservation, PortalOperation,
    PortalPolicy, PortalPolicyError, PortalSession, PortalSessionState, PortalTargetKind,
};

use super::{
    MessageId, PortalActionReceipt, PortalActionState, PortalCapability, PortalCapabilityStatus,
    PortalDescriptor, PortalObservation, PortalPolicyOutcome, PortalTargetKind as WireTargetKind,
};

const JOURNAL_FILE_NAME: &str = "portal-actions.sqlite";
const MAX_OBSERVATION_CHARS: usize = 900_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortalScope {
    workspace_id: u64,
    floor_id: Option<u64>,
    node_id: u64,
}

impl PortalScope {
    pub fn new(
        workspace_id: u64,
        floor_id: Option<u64>,
        node_id: u64,
    ) -> Result<Self, PortalServiceError> {
        if workspace_id == 0 || node_id == 0 || floor_id.is_some_and(|id| id == 0) {
            return Err(PortalServiceError::InvalidScope);
        }
        Ok(Self {
            workspace_id,
            floor_id,
            node_id,
        })
    }

    pub const fn workspace_id(self) -> u64 {
        self.workspace_id
    }

    pub const fn floor_id(self) -> Option<u64> {
        self.floor_id
    }

    pub const fn node_id(self) -> u64 {
        self.node_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalInspection {
    pub descriptor: PortalDescriptor,
    pub capabilities: Vec<PortalCapability>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalObservationResult {
    pub observation: PortalObservation,
    pub core: CoreObservation,
}

pub struct PortalDispatcher {
    portals: BTreeMap<u64, PortalEntry>,
    policy: PortalPolicy,
    journal: PortalActionJournal,
}

struct PortalEntry {
    scope: PortalScope,
    config: PortalConfig,
    session: PortalSession,
    capabilities: PortalCapabilities,
    attached_agents: BTreeSet<u64>,
    backend: Box<dyn ErasedPortalBackend>,
}

trait ErasedPortalBackend: Send {
    fn connect(
        &mut self,
        config: &PortalConfig,
        session: &mut PortalSession,
    ) -> Result<(), PortalServiceError>;

    fn observe(
        &mut self,
        session: &mut PortalSession,
    ) -> Result<CoreObservation, PortalServiceError>;

    fn execute(
        &mut self,
        session: &mut PortalSession,
        action: &PortalAction,
    ) -> Result<(), PortalServiceError>;

    fn close(&mut self, session: &mut PortalSession) -> Result<(), PortalServiceError>;
}

struct BackendAdapter<B>(B);

impl<B> ErasedPortalBackend for BackendAdapter<B>
where
    B: PortalBackend + Send,
    B::Error: Send + Sync,
{
    fn connect(
        &mut self,
        config: &PortalConfig,
        session: &mut PortalSession,
    ) -> Result<(), PortalServiceError> {
        self.0
            .connect(config, session)
            .map_err(|error| PortalServiceError::Backend(error.to_string()))
    }

    fn observe(
        &mut self,
        session: &mut PortalSession,
    ) -> Result<CoreObservation, PortalServiceError> {
        self.0
            .observe(session)
            .map_err(|error| PortalServiceError::Backend(error.to_string()))
    }

    fn execute(
        &mut self,
        session: &mut PortalSession,
        action: &PortalAction,
    ) -> Result<(), PortalServiceError> {
        self.0
            .execute(session, action)
            .map_err(|error| PortalServiceError::Backend(error.to_string()))
    }

    fn close(&mut self, session: &mut PortalSession) -> Result<(), PortalServiceError> {
        self.0
            .close(session)
            .map_err(|error| PortalServiceError::Backend(error.to_string()))
    }
}

impl PortalDispatcher {
    pub fn open(data_directory: impl AsRef<Path>) -> Result<Self, PortalServiceError> {
        Ok(Self {
            portals: BTreeMap::new(),
            policy: PortalPolicy::new(),
            journal: PortalActionJournal::open(data_directory.as_ref().join(JOURNAL_FILE_NAME))?,
        })
    }

    pub fn register<B>(
        &mut self,
        portal_id: u64,
        scope: PortalScope,
        config: PortalConfig,
        backend: B,
    ) -> Result<(), PortalServiceError>
    where
        B: PortalBackend + Send + 'static,
        B::Error: Send + Sync,
    {
        if portal_id == 0 {
            return Err(PortalServiceError::InvalidPortalId);
        }
        if self.portals.contains_key(&portal_id) {
            return Err(PortalServiceError::DuplicatePortal(portal_id));
        }
        let capabilities = capabilities_for(config.target().kind());
        self.portals.insert(
            portal_id,
            PortalEntry {
                scope,
                config,
                session: PortalSession::new(1),
                capabilities,
                attached_agents: BTreeSet::new(),
                backend: Box::new(BackendAdapter(backend)),
            },
        );
        Ok(())
    }

    pub fn attach_agent(
        &mut self,
        portal_id: u64,
        agent_id: u64,
    ) -> Result<(), PortalServiceError> {
        if agent_id == 0 {
            return Err(PortalServiceError::InvalidAgentId);
        }
        self.entry_mut(portal_id)?.attached_agents.insert(agent_id);
        Ok(())
    }

    pub fn detach_agent(&mut self, portal_id: u64, agent_id: u64) {
        if let Some(entry) = self.portals.get_mut(&portal_id) {
            entry.attached_agents.remove(&agent_id);
        }
        self.policy.revoke_agent(agent_id);
    }

    pub fn connect(&mut self, portal_id: u64) -> Result<(), PortalServiceError> {
        let entry = self.entry_mut(portal_id)?;
        entry.backend.connect(&entry.config, &mut entry.session)
    }

    pub fn close(&mut self, portal_id: u64) -> Result<(), PortalServiceError> {
        let entry = self.entry_mut(portal_id)?;
        if !matches!(entry.session.state(), PortalSessionState::Disconnected) {
            entry.backend.close(&mut entry.session)?;
        }
        self.policy.revoke_portal(portal_id);
        Ok(())
    }

    pub fn list(
        &self,
        workspace_id: u64,
        agent_id: u64,
    ) -> Result<Vec<PortalDescriptor>, PortalServiceError> {
        let mut portals = Vec::new();
        for (portal_id, entry) in &self.portals {
            if entry.scope.workspace_id() == workspace_id
                && entry.attached_agents.contains(&agent_id)
                && entry.session.state() == PortalSessionState::Connected
            {
                portals.push(self.descriptor(*portal_id, entry));
            }
        }
        Ok(portals)
    }

    pub fn inspect(
        &self,
        workspace_id: u64,
        agent_id: u64,
        portal_id: u64,
    ) -> Result<PortalInspection, PortalServiceError> {
        let entry = self.authorized_entry(workspace_id, agent_id, portal_id)?;
        Ok(PortalInspection {
            descriptor: self.descriptor(portal_id, entry),
            capabilities: capability_report(&entry.capabilities),
        })
    }

    pub fn observe(
        &mut self,
        workspace_id: u64,
        agent_id: u64,
        portal_id: u64,
    ) -> Result<PortalObservationResult, PortalServiceError> {
        let entry = self.authorized_entry_mut(workspace_id, agent_id, portal_id)?;
        let core = entry.backend.observe(&mut entry.session)?;
        let accessibility = core
            .accessibility()
            .map(|snapshot| snapshot.json().to_owned())
            .filter(|json| json.chars().count() <= MAX_OBSERVATION_CHARS);
        let observation = PortalObservation {
            portal_id,
            revision: core.revision(),
            accessibility,
            frame_available: core.frame().is_some(),
        };
        Ok(PortalObservationResult { observation, core })
    }

    pub fn request_action(
        &mut self,
        workspace_id: u64,
        agent_id: u64,
        action_id: MessageId,
        portal_id: u64,
        action: PortalAction,
    ) -> Result<PortalActionReceipt, PortalServiceError> {
        self.request_action_at(
            workspace_id,
            agent_id,
            action_id,
            portal_id,
            action,
            now_ms(),
        )
    }

    pub fn request_action_at(
        &mut self,
        workspace_id: u64,
        agent_id: u64,
        action_id: MessageId,
        portal_id: u64,
        action: PortalAction,
        now_ms: u64,
    ) -> Result<PortalActionReceipt, PortalServiceError> {
        let entry = self.authorized_entry(workspace_id, agent_id, portal_id)?;
        let target = action_target(&action, entry.config.target().selector());
        let request = PolicyRequest::new(
            agent_id,
            portal_id,
            operation_for(&action),
            target,
            entry.capabilities.clone(),
        )
        .map_err(PortalServiceError::Policy)?;
        let fingerprint = action_fingerprint(&action);

        if let Some(existing) = self.journal.receipt(workspace_id, &action_id)? {
            if existing.portal_id == portal_id && existing.action_id == action_id {
                return Ok(PortalActionReceipt {
                    duplicate: true,
                    ..existing
                });
            }
            return Err(PortalServiceError::ActionConflict(action_id.to_string()));
        }

        let decision = self.policy.evaluate(&request, now_ms);
        match decision {
            PolicyDecision::ApprovalRequired {
                approval_id,
                reason,
            } => {
                let receipt = self.journal.insert_intent(JournalIntent {
                    workspace_id,
                    agent_id,
                    portal_id,
                    action_id: &action_id,
                    fingerprint: &fingerprint,
                    state: "awaiting_approval",
                    policy: "approval_required",
                    policy_detail: Some(format!("{approval_id}:{reason}")),
                    created_at_ms: now_ms,
                })?;
                Ok(receipt)
            }
            PolicyDecision::Denied { reason } => self.journal.insert_intent(JournalIntent {
                workspace_id,
                agent_id,
                portal_id,
                action_id: &action_id,
                fingerprint: &fingerprint,
                state: "failed",
                policy: "denied",
                policy_detail: Some(reason),
                created_at_ms: now_ms,
            }),
            PolicyDecision::Allowed { .. } => {
                self.journal.insert_intent(JournalIntent {
                    workspace_id,
                    agent_id,
                    portal_id,
                    action_id: &action_id,
                    fingerprint: &fingerprint,
                    state: "queued",
                    policy: "allowed",
                    policy_detail: None,
                    created_at_ms: now_ms,
                })?;
                self.journal
                    .mark_dispatched(workspace_id, &action_id, now_ms)?;
                let entry = self.authorized_entry_mut(workspace_id, agent_id, portal_id)?;
                let result = entry.backend.execute(&mut entry.session, &action);
                let (state, outcome) = match result {
                    Ok(()) => ("completed", None),
                    Err(error) => ("failed", Some(error.to_string())),
                };
                self.journal
                    .finish(workspace_id, &action_id, state, outcome, now_ms)?;
                self.journal
                    .receipt(workspace_id, &action_id)?
                    .ok_or_else(|| PortalServiceError::MissingReceipt(action_id.to_string()))
            }
        }
    }

    pub fn result(
        &self,
        workspace_id: u64,
        agent_id: u64,
        action_id: &MessageId,
    ) -> Result<PortalActionReceipt, PortalServiceError> {
        let receipt = self
            .journal
            .receipt(workspace_id, action_id)?
            .ok_or_else(|| PortalServiceError::UnknownAction(action_id.to_string()))?;
        let entry = self.entry(receipt.portal_id)?;
        if !entry.attached_agents.contains(&agent_id) {
            return Err(PortalServiceError::AgentNotAttached {
                portal_id: receipt.portal_id,
                agent_id,
            });
        }
        Ok(receipt)
    }

    pub fn set_policy_rule(&mut self, operation: PortalOperation, rule: crate::portal::PolicyRule) {
        self.policy.set_rule(operation, rule);
    }

    fn entry(&self, portal_id: u64) -> Result<&PortalEntry, PortalServiceError> {
        self.portals
            .get(&portal_id)
            .ok_or(PortalServiceError::UnknownPortal(portal_id))
    }

    fn entry_mut(&mut self, portal_id: u64) -> Result<&mut PortalEntry, PortalServiceError> {
        self.portals
            .get_mut(&portal_id)
            .ok_or(PortalServiceError::UnknownPortal(portal_id))
    }

    fn authorized_entry(
        &self,
        workspace_id: u64,
        agent_id: u64,
        portal_id: u64,
    ) -> Result<&PortalEntry, PortalServiceError> {
        let entry = self.entry(portal_id)?;
        authorize_entry(entry, workspace_id, agent_id, portal_id)
    }

    fn authorized_entry_mut(
        &mut self,
        workspace_id: u64,
        agent_id: u64,
        portal_id: u64,
    ) -> Result<&mut PortalEntry, PortalServiceError> {
        let entry = self.entry_mut(portal_id)?;
        authorize_entry(entry, workspace_id, agent_id, portal_id)?;
        Ok(entry)
    }

    fn descriptor(&self, portal_id: u64, entry: &PortalEntry) -> PortalDescriptor {
        PortalDescriptor {
            id: portal_id,
            target_kind: wire_target_kind(entry.config.target().kind()),
            target: entry.config.target().selector().to_owned(),
            state: format!("{:?}", entry.session.state()).to_ascii_lowercase(),
            generation: entry.session.generation(),
        }
    }
}

fn authorize_entry(
    entry: &PortalEntry,
    workspace_id: u64,
    agent_id: u64,
    portal_id: u64,
) -> Result<&PortalEntry, PortalServiceError> {
    if entry.scope.workspace_id() != workspace_id {
        return Err(PortalServiceError::WrongWorkspace {
            portal_id,
            workspace_id,
        });
    }
    if !entry.attached_agents.contains(&agent_id) {
        return Err(PortalServiceError::AgentNotAttached {
            portal_id,
            agent_id,
        });
    }
    if entry.session.state() != PortalSessionState::Connected {
        return Err(PortalServiceError::NotConnected(portal_id));
    }
    Ok(entry)
}

fn capabilities_for(kind: PortalTargetKind) -> PortalCapabilities {
    let mut capabilities = PortalCapabilities::browser_defaults();
    if kind != PortalTargetKind::Browser {
        for operation in [
            PortalOperation::Observe,
            PortalOperation::Screenshot,
            PortalOperation::Navigate,
            PortalOperation::Input,
            PortalOperation::CoordinateFallback,
            PortalOperation::Upload,
            PortalOperation::Download,
            PortalOperation::Clipboard,
            PortalOperation::SensitivePermission,
        ] {
            capabilities.set_status(
                operation,
                CapabilityStatus::Unavailable {
                    reason: "device adapter is not registered".to_owned(),
                },
            );
        }
    }
    capabilities
}

fn capability_report(capabilities: &PortalCapabilities) -> Vec<PortalCapability> {
    [
        PortalOperation::Observe,
        PortalOperation::Screenshot,
        PortalOperation::Navigate,
        PortalOperation::Input,
        PortalOperation::CoordinateFallback,
        PortalOperation::Upload,
        PortalOperation::Download,
        PortalOperation::Clipboard,
        PortalOperation::SensitivePermission,
    ]
    .into_iter()
    .map(|operation| PortalCapability {
        operation: operation_name(operation).to_owned(),
        status: capability_status(capabilities.status(operation)),
    })
    .collect()
}

fn capability_status(status: &CapabilityStatus) -> PortalCapabilityStatus {
    match status {
        CapabilityStatus::Supported => PortalCapabilityStatus::Supported,
        CapabilityStatus::Unavailable { reason } => PortalCapabilityStatus::Unavailable {
            reason: reason.clone(),
        },
        CapabilityStatus::PermissionRequired { reason } => {
            PortalCapabilityStatus::PermissionRequired {
                reason: reason.clone(),
            }
        }
    }
}

fn operation_name(operation: PortalOperation) -> &'static str {
    match operation {
        PortalOperation::Observe => "observe",
        PortalOperation::Screenshot => "screenshot",
        PortalOperation::Navigate => "navigate",
        PortalOperation::Input => "input",
        PortalOperation::CoordinateFallback => "coordinate_fallback",
        PortalOperation::Upload => "upload",
        PortalOperation::Download => "download",
        PortalOperation::Clipboard => "clipboard",
        PortalOperation::SensitivePermission => "sensitive_permission",
    }
}

fn wire_target_kind(kind: PortalTargetKind) -> WireTargetKind {
    match kind {
        PortalTargetKind::Browser => WireTargetKind::Browser,
        PortalTargetKind::Android => WireTargetKind::Android,
        PortalTargetKind::Ios => WireTargetKind::Ios,
    }
}

fn action_target<'a>(action: &'a PortalAction, current_target: &'a str) -> &'a str {
    match action {
        PortalAction::Navigate(target) => target,
        PortalAction::Click(_) | PortalAction::TypeText { .. } | PortalAction::Scroll { .. } => {
            current_target
        }
    }
}

fn operation_for(action: &PortalAction) -> PortalOperation {
    match action {
        PortalAction::Navigate(_) => PortalOperation::Navigate,
        PortalAction::Click(_) | PortalAction::TypeText { .. } | PortalAction::Scroll { .. } => {
            PortalOperation::Input
        }
    }
}

fn action_fingerprint(action: &PortalAction) -> String {
    blake3::hash(format!("{action:?}").as_bytes())
        .to_hex()
        .to_string()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[derive(Debug)]
struct PortalActionJournal {
    connection: Connection,
}

struct JournalIntent<'a> {
    workspace_id: u64,
    agent_id: u64,
    portal_id: u64,
    action_id: &'a MessageId,
    fingerprint: &'a str,
    state: &'a str,
    policy: &'a str,
    policy_detail: Option<String>,
    created_at_ms: u64,
}

impl PortalActionJournal {
    fn open(path: impl AsRef<Path>) -> Result<Self, PortalServiceError> {
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
        Ok(Self { connection })
    }

    fn insert_intent(
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

    fn mark_dispatched(
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

    fn finish(
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

    fn receipt(
        &self,
        workspace_id: u64,
        action_id: &MessageId,
    ) -> Result<Option<PortalActionReceipt>, PortalServiceError> {
        self.record(workspace_id, action_id)?
            .map(|record| record.into_receipt(false))
            .transpose()
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

#[derive(Debug)]
pub enum PortalServiceError {
    InvalidPortalId,
    InvalidAgentId,
    InvalidScope,
    DuplicatePortal(u64),
    UnknownPortal(u64),
    WrongWorkspace { portal_id: u64, workspace_id: u64 },
    AgentNotAttached { portal_id: u64, agent_id: u64 },
    NotConnected(u64),
    Backend(String),
    Session(String),
    Policy(PortalPolicyError),
    ActionConflict(String),
    UnknownAction(String),
    MissingReceipt(String),
    Journal(String),
}

impl Display for PortalServiceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPortalId => formatter.write_str("portal ID must be greater than zero"),
            Self::InvalidAgentId => formatter.write_str("agent ID must be greater than zero"),
            Self::InvalidScope => formatter.write_str("portal scope IDs must be positive"),
            Self::DuplicatePortal(id) => write!(formatter, "portal {id} is already registered"),
            Self::UnknownPortal(id) => write!(formatter, "portal {id} is not registered"),
            Self::WrongWorkspace {
                portal_id,
                workspace_id,
            } => write!(
                formatter,
                "portal {portal_id} is not in workspace {workspace_id}"
            ),
            Self::AgentNotAttached {
                portal_id,
                agent_id,
            } => {
                write!(
                    formatter,
                    "agent {agent_id} is not connected to portal {portal_id}"
                )
            }
            Self::NotConnected(id) => write!(formatter, "portal {id} is disconnected"),
            Self::Backend(message) => write!(formatter, "portal backend failed: {message}"),
            Self::Session(message) => write!(formatter, "portal session failed: {message}"),
            Self::Policy(error) => error.fmt(formatter),
            Self::ActionConflict(id) => {
                write!(
                    formatter,
                    "portal action ID {id} conflicts with an existing action"
                )
            }
            Self::UnknownAction(id) => write!(formatter, "portal action {id} does not exist"),
            Self::MissingReceipt(id) => write!(formatter, "portal action {id} has no receipt"),
            Self::Journal(message) => write!(formatter, "portal action journal failed: {message}"),
        }
    }
}

impl Error for PortalServiceError {}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tempfile::TempDir;

    use super::*;
    use crate::portal::{
        PortalElementRef, PortalFrame, PortalFrameEncoding, PortalObservation as CoreObservation,
        PortalViewport,
    };

    #[derive(Clone, Default)]
    struct FakeBackend {
        executed: Arc<Mutex<Vec<PortalAction>>>,
    }

    #[derive(Debug)]
    struct FakeError;

    impl Display for FakeError {
        fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
            formatter.write_str("fake backend error")
        }
    }

    impl Error for FakeError {}

    impl PortalBackend for FakeBackend {
        type Error = FakeError;

        fn connect(
            &mut self,
            _config: &PortalConfig,
            session: &mut PortalSession,
        ) -> Result<(), Self::Error> {
            session.begin_connect().map_err(|_| FakeError)?;
            session.connected().map_err(|_| FakeError)
        }

        fn observe(&mut self, session: &mut PortalSession) -> Result<CoreObservation, Self::Error> {
            let revision = session.observe().map_err(|_| FakeError)?;
            let observation =
                CoreObservation::new(revision, PortalCapabilities::browser_defaults()).with_frame(
                    PortalFrame::new(
                        revision,
                        PortalViewport::new(10, 10).unwrap(),
                        PortalFrameEncoding::Png,
                        vec![1],
                    )
                    .unwrap(),
                );
            session
                .record_observation(observation.clone())
                .map_err(|_| FakeError)?;
            Ok(observation)
        }

        fn execute(
            &mut self,
            _session: &mut PortalSession,
            action: &PortalAction,
        ) -> Result<(), Self::Error> {
            self.executed.lock().unwrap().push(action.clone());
            Ok(())
        }

        fn close(&mut self, session: &mut PortalSession) -> Result<(), Self::Error> {
            session.begin_close().map_err(|_| FakeError)?;
            session.closed();
            Ok(())
        }
    }

    fn dispatcher(temp: &TempDir) -> (PortalDispatcher, Arc<Mutex<Vec<PortalAction>>>) {
        let executed = Arc::new(Mutex::new(Vec::new()));
        let backend = FakeBackend {
            executed: Arc::clone(&executed),
        };
        let config = PortalConfig::browser("https://example.test").unwrap();
        let mut dispatcher = PortalDispatcher::open(temp.path()).unwrap();
        dispatcher
            .register(
                10,
                PortalScope::new(4, Some(2), 99).unwrap(),
                config,
                backend,
            )
            .unwrap();
        dispatcher.attach_agent(10, 7).unwrap();
        dispatcher.connect(10).unwrap();
        (dispatcher, executed)
    }

    #[test]
    fn access_is_scoped_to_workspace_and_attached_agent() {
        let temp = TempDir::new().unwrap();
        let (dispatcher, _) = dispatcher(&temp);
        assert!(matches!(
            dispatcher.list(5, 7),
            Ok(portals) if portals.is_empty()
        ));
        assert!(matches!(
            dispatcher.inspect(4, 8, 10),
            Err(PortalServiceError::AgentNotAttached { .. })
        ));
    }

    #[test]
    fn allowed_observation_and_approval_required_actions_are_journaled() {
        let temp = TempDir::new().unwrap();
        let (mut dispatcher, executed) = dispatcher(&temp);
        let observation = dispatcher.observe(4, 7, 10).unwrap();
        assert!(observation.observation.frame_available);

        let click = PortalAction::Click(
            PortalElementRef::new(observation.observation.revision, "submit").unwrap(),
        );
        let receipt = dispatcher
            .request_action_at(4, 7, MessageId::new("action-1").unwrap(), 10, click, 100)
            .unwrap();
        assert_eq!(receipt.state, PortalActionState::AwaitingApproval);
        assert!(executed.lock().unwrap().is_empty());

        let duplicate = dispatcher
            .request_action_at(
                4,
                7,
                MessageId::new("action-1").unwrap(),
                10,
                PortalAction::Click(
                    PortalElementRef::new(observation.observation.revision, "submit").unwrap(),
                ),
                101,
            )
            .unwrap();
        assert!(duplicate.duplicate);
    }

    #[test]
    fn restart_marks_dispatched_receipts_unknown() {
        let temp = TempDir::new().unwrap();
        let journal_path = temp.path().join(JOURNAL_FILE_NAME);
        let journal = PortalActionJournal::open(&journal_path).unwrap();
        let action_id = MessageId::new("action-1").unwrap();
        journal
            .insert_intent(JournalIntent {
                workspace_id: 4,
                agent_id: 7,
                portal_id: 10,
                action_id: &action_id,
                fingerprint: "fingerprint",
                state: "queued",
                policy: "allowed",
                policy_detail: None,
                created_at_ms: 1,
            })
            .unwrap();
        journal.mark_dispatched(4, &action_id, 2).unwrap();
        drop(journal);

        let journal = PortalActionJournal::open(&journal_path).unwrap();
        let receipt = journal.receipt(4, &action_id).unwrap().unwrap();
        assert_eq!(receipt.state, PortalActionState::Unknown);
        assert_eq!(
            receipt.outcome.as_deref(),
            Some("execution outcome unknown after restart")
        );
    }
}
