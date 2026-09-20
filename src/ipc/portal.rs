use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;

use crate::portal::{
    CapabilityStatus, PolicyDecision, PolicyRequest, PortalAction, PortalBackend,
    PortalCapabilities, PortalConfig, PortalObservation as CoreObservation, PortalOperation,
    PortalPolicy, PortalPolicyError, PortalSession, PortalSessionState, PortalTargetKind,
};

use super::protocol::{MAX_PORTAL_FRAME_CHUNK_BYTES, PortalFrameChunk};
use super::{
    MessageId, PortalActionReceipt, PortalCapability, PortalCapabilityStatus, PortalDescriptor,
    PortalObservation, PortalTargetKind as WireTargetKind,
};

mod journal;
use journal::{JournalIntent, PortalActionJournal};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalPendingApproval {
    pub workspace_id: u64,
    pub action_id: MessageId,
    pub portal_id: u64,
    pub agent_id: u64,
    pub approval_id: u64,
    pub operation: String,
    pub target: String,
    pub reason: String,
}

pub struct PortalDispatcher {
    portals: BTreeMap<u64, PortalEntry>,
    next_portal_id: u64,
    policy: PortalPolicy,
    journal: PortalActionJournal,
    pending_actions: BTreeMap<(u64, MessageId), PendingAction>,
}

#[derive(Clone)]
struct PendingAction {
    agent_id: u64,
    portal_id: u64,
    approval_id: u64,
    reason: String,
    request: PolicyRequest,
    action: PortalAction,
}

struct ActionDispatch {
    agent_id: u64,
    portal_id: u64,
    request: PolicyRequest,
    action: PortalAction,
}

struct PortalEntry {
    scope: PortalScope,
    config: PortalConfig,
    runtime_target: String,
    session: PortalSession,
    capabilities: PortalCapabilities,
    attached_agents: BTreeSet<u64>,
    backend: Box<dyn ErasedPortalBackend>,
}

trait ErasedPortalBackend: Send {
    fn policy_target(&mut self, config: &PortalConfig) -> Result<String, PortalServiceError>;

    fn connect(
        &mut self,
        config: &PortalConfig,
        session: &mut PortalSession,
    ) -> Result<(), PortalServiceError>;

    fn observe(
        &mut self,
        session: &mut PortalSession,
    ) -> Result<CoreObservation, PortalServiceError>;

    fn capture_frame(
        &mut self,
        session: &PortalSession,
    ) -> Result<Option<crate::portal::PortalFrame>, PortalServiceError>;

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
    fn policy_target(&mut self, config: &PortalConfig) -> Result<String, PortalServiceError> {
        self.0
            .policy_target(config)
            .map_err(|error| PortalServiceError::Backend(error.to_string()))
    }

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

    fn capture_frame(
        &mut self,
        session: &PortalSession,
    ) -> Result<Option<crate::portal::PortalFrame>, PortalServiceError> {
        self.0
            .capture_frame(session)
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
            next_portal_id: 1,
            policy: PortalPolicy::new(),
            journal: PortalActionJournal::open(data_directory.as_ref().join(JOURNAL_FILE_NAME))?,
            pending_actions: BTreeMap::new(),
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
        let capabilities = backend.capabilities();
        let runtime_target = config.target().selector().to_owned();
        self.portals.insert(
            portal_id,
            PortalEntry {
                scope,
                config,
                runtime_target,
                session: PortalSession::new(1),
                capabilities,
                attached_agents: BTreeSet::new(),
                backend: Box::new(BackendAdapter(backend)),
            },
        );
        Ok(())
    }

    pub fn register_auto<B>(
        &mut self,
        scope: PortalScope,
        config: PortalConfig,
        backend: B,
    ) -> Result<u64, PortalServiceError>
    where
        B: PortalBackend + Send + 'static,
        B::Error: Send + Sync,
    {
        let start = self.next_portal_id;
        loop {
            let portal_id = self.next_portal_id;
            self.next_portal_id = self.next_portal_id.wrapping_add(1).max(1);
            if !self.portals.contains_key(&portal_id) {
                self.register(portal_id, scope, config, backend)?;
                return Ok(portal_id);
            }
            if self.next_portal_id == start {
                return Err(PortalServiceError::PortalIdsExhausted);
            }
        }
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
        self.policy.revoke_connection(agent_id, portal_id);
    }

    pub fn replace_agents(
        &mut self,
        portal_id: u64,
        agent_ids: impl IntoIterator<Item = u64>,
    ) -> Result<(), PortalServiceError> {
        let replacement = agent_ids.into_iter().collect::<BTreeSet<_>>();
        if replacement.contains(&0) {
            return Err(PortalServiceError::InvalidAgentId);
        }
        let removed = {
            let entry = self.entry_mut(portal_id)?;
            let removed = entry
                .attached_agents
                .difference(&replacement)
                .copied()
                .collect::<BTreeSet<_>>();
            entry.attached_agents = replacement;
            removed
        };
        for agent_id in &removed {
            self.policy.revoke_connection(*agent_id, portal_id);
        }
        let pending = self
            .pending_actions
            .iter()
            .filter(|(_, action)| {
                action.portal_id == portal_id && removed.contains(&action.agent_id)
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for (workspace_id, action_id) in pending {
            self.pending_actions
                .remove(&(workspace_id, action_id.clone()));
            self.journal.finish(
                workspace_id,
                &action_id,
                "failed",
                Some("agent disconnected from portal before approval".to_owned()),
                now_ms(),
            )?;
        }
        Ok(())
    }

    pub fn connect(&mut self, portal_id: u64) -> Result<(), PortalServiceError> {
        let entry = self.entry_mut(portal_id)?;
        if entry.session.state() == PortalSessionState::Connected {
            return Ok(());
        }
        if entry.session.state() == PortalSessionState::Closed {
            entry.session = PortalSession::new(entry.session.generation().wrapping_add(1));
        }
        entry.backend.connect(&entry.config, &mut entry.session)
    }

    pub fn close(&mut self, portal_id: u64) -> Result<(), PortalServiceError> {
        {
            let entry = self.entry_mut(portal_id)?;
            if !matches!(
                entry.session.state(),
                PortalSessionState::Disconnected | PortalSessionState::Closed
            ) {
                entry.backend.close(&mut entry.session)?;
            }
        }
        let pending: Vec<_> = self
            .pending_actions
            .iter()
            .filter(|(_, action)| action.portal_id == portal_id)
            .map(|(key, _)| key.clone())
            .collect();
        for (workspace_id, action_id) in pending {
            self.pending_actions
                .remove(&(workspace_id, action_id.clone()));
            self.journal.finish(
                workspace_id,
                &action_id,
                "failed",
                Some("portal closed before approval".to_owned()),
                now_ms(),
            )?;
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
        observe_entry(portal_id, entry)
    }

    pub fn frame_chunk(
        &self,
        workspace_id: u64,
        agent_id: u64,
        portal_id: u64,
        observation_revision: u64,
        offset: u64,
        max_bytes: u32,
    ) -> Result<PortalFrameChunk, PortalServiceError> {
        let entry = self.authorized_entry(workspace_id, agent_id, portal_id)?;
        let observation = entry
            .session
            .latest_observation()
            .ok_or(PortalServiceError::FrameUnavailable(portal_id))?;
        if observation.revision() != observation_revision {
            return Err(PortalServiceError::StaleFrame {
                expected: observation.revision(),
                found: observation_revision,
            });
        }
        let frame = observation
            .frame()
            .ok_or(PortalServiceError::FrameUnavailable(portal_id))?;
        let offset =
            usize::try_from(offset).map_err(|_| PortalServiceError::InvalidFrameOffset(offset))?;
        if offset > frame.bytes().len() {
            return Err(PortalServiceError::InvalidFrameOffset(
                u64::try_from(offset).unwrap_or(u64::MAX),
            ));
        }
        let chunk_size = usize::try_from(max_bytes.min(MAX_PORTAL_FRAME_CHUNK_BYTES))
            .expect("frame chunk limit fits in usize");
        let end = offset.saturating_add(chunk_size).min(frame.bytes().len());
        let viewport = frame.viewport();
        Ok(PortalFrameChunk {
            portal_id,
            revision: frame.revision(),
            width: viewport.width(),
            height: viewport.height(),
            encoding: match frame.encoding() {
                crate::portal::PortalFrameEncoding::Png => "png",
                crate::portal::PortalFrameEncoding::Jpeg => "jpeg",
                crate::portal::PortalFrameEncoding::Webp => "webp",
            }
            .to_owned(),
            offset: u64::try_from(offset).expect("validated frame offset fits in u64"),
            total_bytes: u64::try_from(frame.bytes().len()).expect("frame length fits in u64"),
            data_base64: base64::engine::general_purpose::STANDARD
                .encode(&frame.bytes()[offset..end]),
            complete: end == frame.bytes().len(),
        })
    }

    pub fn observe_local(
        &mut self,
        portal_id: u64,
    ) -> Result<PortalObservationResult, PortalServiceError> {
        let entry = self.entry_mut(portal_id)?;
        if entry.session.state() != PortalSessionState::Connected {
            return Err(PortalServiceError::NotConnected(portal_id));
        }
        observe_entry(portal_id, entry)
    }

    pub fn capture_frame_local(
        &mut self,
        portal_id: u64,
    ) -> Result<Option<crate::portal::PortalFrame>, PortalServiceError> {
        let entry = self.entry_mut(portal_id)?;
        if entry.session.state() != PortalSessionState::Connected {
            return Err(PortalServiceError::NotConnected(portal_id));
        }
        entry.backend.capture_frame(&entry.session)
    }

    pub fn execute_local(
        &mut self,
        portal_id: u64,
        action: PortalAction,
    ) -> Result<(), PortalServiceError> {
        let entry = self.entry_mut(portal_id)?;
        if entry.session.state() != PortalSessionState::Connected {
            return Err(PortalServiceError::NotConnected(portal_id));
        }
        entry.backend.execute(&mut entry.session, &action)
    }

    pub fn unregister(&mut self, portal_id: u64) -> Result<(), PortalServiceError> {
        self.close(portal_id)?;
        self.portals.remove(&portal_id);
        Ok(())
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
        self.authorized_entry(workspace_id, agent_id, portal_id)?;
        let fingerprint = action_fingerprint(&action);
        if let Some(existing) = self.journal.duplicate_receipt(
            workspace_id,
            &action_id,
            agent_id,
            portal_id,
            &fingerprint,
        )? {
            return Ok(existing);
        }

        let current_target = self.refresh_runtime_target(portal_id)?;
        let entry = self.authorized_entry(workspace_id, agent_id, portal_id)?;
        let target = action_target(&action, &current_target);
        let request = PolicyRequest::new(
            agent_id,
            portal_id,
            operation_for(&action),
            target,
            entry.capabilities.clone(),
        )
        .map_err(PortalServiceError::Policy)?;

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
                self.pending_actions.insert(
                    (workspace_id, action_id.clone()),
                    PendingAction {
                        agent_id,
                        portal_id,
                        approval_id,
                        reason,
                        request,
                        action,
                    },
                );
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
                self.dispatch_action(
                    workspace_id,
                    action_id,
                    ActionDispatch {
                        agent_id,
                        portal_id,
                        request,
                        action,
                    },
                    now_ms,
                )
            }
        }
    }

    pub fn approve_action(
        &mut self,
        workspace_id: u64,
        action_id: &MessageId,
        approval_id: u64,
        lifetime_ms: u64,
    ) -> Result<PortalActionReceipt, PortalServiceError> {
        self.approve_action_at(workspace_id, action_id, approval_id, lifetime_ms, now_ms())
    }

    pub fn pending_approvals(&self, workspace_id: u64) -> Vec<PortalPendingApproval> {
        self.pending_actions
            .iter()
            .filter(|((pending_workspace_id, _), _)| *pending_workspace_id == workspace_id)
            .map(|((_, action_id), pending)| PortalPendingApproval {
                workspace_id,
                action_id: action_id.clone(),
                portal_id: pending.portal_id,
                agent_id: pending.agent_id,
                approval_id: pending.approval_id,
                operation: operation_name(pending.request.operation()).to_owned(),
                target: pending.request.target().to_owned(),
                reason: pending.reason.clone(),
            })
            .collect()
    }

    pub fn reject_action(
        &mut self,
        workspace_id: u64,
        action_id: &MessageId,
        approval_id: u64,
        reason: &str,
    ) -> Result<PortalActionReceipt, PortalServiceError> {
        let key = (workspace_id, action_id.clone());
        let pending = self
            .pending_actions
            .get(&key)
            .ok_or_else(|| PortalServiceError::PendingActionUnavailable(action_id.to_string()))?;
        if pending.approval_id != approval_id {
            return Err(PortalServiceError::ApprovalMismatch {
                expected: pending.approval_id,
                found: approval_id,
            });
        }
        self.policy
            .reject(approval_id)
            .map_err(PortalServiceError::Policy)?;
        self.pending_actions.remove(&key);
        self.journal
            .mark_rejected(workspace_id, action_id, reason, now_ms())?;
        self.journal
            .receipt(workspace_id, action_id)?
            .ok_or_else(|| PortalServiceError::MissingReceipt(action_id.to_string()))
    }

    pub fn approve_action_at(
        &mut self,
        workspace_id: u64,
        action_id: &MessageId,
        approval_id: u64,
        lifetime_ms: u64,
        now_ms: u64,
    ) -> Result<PortalActionReceipt, PortalServiceError> {
        let key = (workspace_id, action_id.clone());
        let pending =
            self.pending_actions.get(&key).cloned().ok_or_else(|| {
                PortalServiceError::PendingActionUnavailable(action_id.to_string())
            })?;
        if pending.approval_id != approval_id {
            return Err(PortalServiceError::ApprovalMismatch {
                expected: pending.approval_id,
                found: approval_id,
            });
        }

        self.authorized_entry(workspace_id, pending.agent_id, pending.portal_id)?;
        let current_target = self.refresh_runtime_target(pending.portal_id)?;
        let entry = self.authorized_entry(workspace_id, pending.agent_id, pending.portal_id)?;
        let current_request = PolicyRequest::new(
            pending.agent_id,
            pending.portal_id,
            operation_for(&pending.action),
            action_target(&pending.action, &current_target),
            entry.capabilities.clone(),
        )
        .map_err(PortalServiceError::Policy)?;
        if current_request != pending.request {
            return Err(PortalServiceError::Policy(PortalPolicyError::StaleApproval));
        }
        self.policy
            .approve(approval_id, &current_request, now_ms, lifetime_ms)
            .map_err(PortalServiceError::Policy)?;
        self.journal.mark_approved(workspace_id, action_id)?;
        self.pending_actions.remove(&key);
        self.dispatch_action(
            workspace_id,
            action_id.clone(),
            ActionDispatch {
                agent_id: pending.agent_id,
                portal_id: pending.portal_id,
                request: current_request,
                action: pending.action,
            },
            now_ms,
        )
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

    fn dispatch_action(
        &mut self,
        workspace_id: u64,
        action_id: MessageId,
        dispatch: ActionDispatch,
        now_ms: u64,
    ) -> Result<PortalActionReceipt, PortalServiceError> {
        let ActionDispatch {
            agent_id,
            portal_id,
            request: expected_request,
            action,
        } = dispatch;
        let current_target = self.refresh_runtime_target(portal_id)?;
        let entry = self.authorized_entry(workspace_id, agent_id, portal_id)?;
        let current_request = PolicyRequest::new(
            agent_id,
            portal_id,
            operation_for(&action),
            action_target(&action, &current_target),
            entry.capabilities.clone(),
        )
        .map_err(PortalServiceError::Policy)?;
        if current_request != expected_request {
            self.journal.finish(
                workspace_id,
                &action_id,
                "failed",
                Some("portal target changed before dispatch".to_owned()),
                now_ms,
            )?;
            return self
                .journal
                .receipt(workspace_id, &action_id)?
                .ok_or_else(|| PortalServiceError::MissingReceipt(action_id.to_string()));
        }
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

    fn refresh_runtime_target(&mut self, portal_id: u64) -> Result<String, PortalServiceError> {
        let (target, changed) = {
            let entry = self.entry_mut(portal_id)?;
            let target = entry.backend.policy_target(&entry.config)?;
            let changed = target != entry.runtime_target;
            if changed {
                entry.runtime_target.clone_from(&target);
            }
            (target, changed)
        };
        if changed {
            self.policy.revoke_portal_grants(portal_id);
        }
        Ok(target)
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

fn observe_entry(
    portal_id: u64,
    entry: &mut PortalEntry,
) -> Result<PortalObservationResult, PortalServiceError> {
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
        PortalAction::Click(_)
        | PortalAction::ClickCoordinate { .. }
        | PortalAction::TypeText { .. }
        | PortalAction::TypeFocused { .. }
        | PortalAction::Key { .. }
        | PortalAction::Scroll { .. }
        | PortalAction::ScrollCoordinate { .. } => current_target,
    }
}

fn operation_for(action: &PortalAction) -> PortalOperation {
    match action {
        PortalAction::Navigate(_) => PortalOperation::Navigate,
        PortalAction::ClickCoordinate { .. } | PortalAction::ScrollCoordinate { .. } => {
            PortalOperation::CoordinateFallback
        }
        PortalAction::Click(_)
        | PortalAction::TypeText { .. }
        | PortalAction::TypeFocused { .. }
        | PortalAction::Key { .. }
        | PortalAction::Scroll { .. } => PortalOperation::Input,
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
pub enum PortalServiceError {
    InvalidPortalId,
    InvalidAgentId,
    InvalidScope,
    DuplicatePortal(u64),
    PortalIdsExhausted,
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
    PendingActionUnavailable(String),
    ApprovalMismatch { expected: u64, found: u64 },
    FrameUnavailable(u64),
    StaleFrame { expected: u64, found: u64 },
    InvalidFrameOffset(u64),
    Journal(String),
}

impl Display for PortalServiceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPortalId => formatter.write_str("portal ID must be greater than zero"),
            Self::InvalidAgentId => formatter.write_str("agent ID must be greater than zero"),
            Self::InvalidScope => formatter.write_str("portal scope IDs must be positive"),
            Self::DuplicatePortal(id) => write!(formatter, "portal {id} is already registered"),
            Self::PortalIdsExhausted => formatter.write_str("portal IDs are exhausted"),
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
            Self::PendingActionUnavailable(id) => {
                write!(
                    formatter,
                    "portal action {id} is not awaiting local approval"
                )
            }
            Self::ApprovalMismatch { expected, found } => write!(
                formatter,
                "portal approval changed: expected {expected}, found {found}"
            ),
            Self::FrameUnavailable(id) => {
                write!(
                    formatter,
                    "portal {id} has no frame for its latest observation"
                )
            }
            Self::StaleFrame { expected, found } => write!(
                formatter,
                "portal frame revision is stale: expected {expected}, found {found}"
            ),
            Self::InvalidFrameOffset(offset) => {
                write!(
                    formatter,
                    "portal frame offset {offset} is outside the frame"
                )
            }
            Self::Journal(message) => write!(formatter, "portal action journal failed: {message}"),
        }
    }
}

impl Error for PortalServiceError {}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use tempfile::TempDir;

    use super::*;
    use crate::ipc::{PortalActionState, PortalPolicyOutcome};
    use crate::portal::{
        PortalElementRef, PortalFrame, PortalFrameEncoding, PortalObservation as CoreObservation,
        PortalViewport,
    };

    #[derive(Clone)]
    struct FakeBackend {
        executed: Arc<Mutex<Vec<PortalAction>>>,
        target: Arc<Mutex<String>>,
        target_queries: Arc<Mutex<VecDeque<String>>>,
    }

    impl Default for FakeBackend {
        fn default() -> Self {
            Self {
                executed: Arc::default(),
                target: Arc::new(Mutex::new("https://example.test".to_owned())),
                target_queries: Arc::default(),
            }
        }
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

        fn policy_target(&mut self, _config: &PortalConfig) -> Result<String, Self::Error> {
            if let Some(target) = self.target_queries.lock().unwrap().pop_front() {
                self.target.lock().unwrap().clone_from(&target);
            }
            Ok(self.target.lock().unwrap().clone())
        }

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
                        vec![1, 2, 3, 4],
                    )
                    .unwrap(),
                );
            session
                .record_observation(observation.clone())
                .map_err(|_| FakeError)?;
            Ok(observation)
        }

        fn capture_frame(
            &mut self,
            session: &PortalSession,
        ) -> Result<Option<PortalFrame>, Self::Error> {
            Ok(Some(
                PortalFrame::new(
                    session.observation_revision(),
                    PortalViewport::new(10, 10).unwrap(),
                    PortalFrameEncoding::Png,
                    vec![2],
                )
                .unwrap(),
            ))
        }

        fn execute(
            &mut self,
            _session: &mut PortalSession,
            action: &PortalAction,
        ) -> Result<(), Self::Error> {
            if let PortalAction::Navigate(target) = action {
                self.target.lock().unwrap().clone_from(target);
            }
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
            ..FakeBackend::default()
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
    fn allocated_portal_ids_are_unique_across_workspaces() {
        let temp = TempDir::new().unwrap();
        let mut dispatcher = PortalDispatcher::open(temp.path()).unwrap();
        let first = dispatcher
            .register_auto(
                PortalScope::new(1, None, 1).unwrap(),
                PortalConfig::browser("https://one.example").unwrap(),
                FakeBackend::default(),
            )
            .unwrap();
        let second = dispatcher
            .register_auto(
                PortalScope::new(2, None, 1).unwrap(),
                PortalConfig::browser("https://two.example").unwrap(),
                FakeBackend::default(),
            )
            .unwrap();

        assert_ne!(first, second);
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
        let action_id = MessageId::new("action-1").unwrap();
        let receipt = dispatcher
            .request_action_at(4, 7, action_id.clone(), 10, click, 100)
            .unwrap();
        assert_eq!(receipt.state, PortalActionState::AwaitingApproval);
        assert!(executed.lock().unwrap().is_empty());
        let approval_id = match receipt.policy {
            PortalPolicyOutcome::ApprovalRequired { approval_id, .. } => approval_id,
            policy => panic!("unexpected policy outcome: {policy:?}"),
        };

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

        let approved = dispatcher
            .approve_action_at(4, &action_id, approval_id, 1_000, 102)
            .unwrap();
        assert_eq!(approved.state, PortalActionState::Completed);
        assert_eq!(executed.lock().unwrap().len(), 1);
        assert_eq!(
            dispatcher.result(4, 7, &action_id).unwrap().state,
            PortalActionState::Completed
        );
    }

    #[test]
    fn local_coordinate_input_uses_the_connected_backend() {
        let temp = TempDir::new().unwrap();
        let (mut dispatcher, executed) = dispatcher(&temp);
        let observation = dispatcher.observe_local(10).unwrap();
        let action = PortalAction::ClickCoordinate {
            observation_revision: observation.observation.revision,
            x: 4,
            y: 6,
        };

        dispatcher.execute_local(10, action.clone()).unwrap();

        assert_eq!(executed.lock().unwrap().as_slice(), &[action]);
    }

    #[test]
    fn local_frame_refresh_preserves_agent_observation_revision() {
        let temp = TempDir::new().unwrap();
        let (mut dispatcher, _) = dispatcher(&temp);
        let observation = dispatcher.observe(4, 7, 10).unwrap();

        let frame = dispatcher.capture_frame_local(10).unwrap().unwrap();

        assert_eq!(frame.revision(), observation.observation.revision);
        assert_eq!(
            dispatcher
                .portals
                .get(&10)
                .unwrap()
                .session
                .observation_revision(),
            observation.observation.revision
        );
    }

    #[test]
    fn authenticated_frame_retrieval_is_revision_bound_and_chunked() {
        let temp = TempDir::new().unwrap();
        let (mut dispatcher, _) = dispatcher(&temp);
        let observation = dispatcher.observe(4, 7, 10).unwrap();

        let first = dispatcher
            .frame_chunk(4, 7, 10, observation.observation.revision, 0, 2)
            .unwrap();
        let second = dispatcher
            .frame_chunk(4, 7, 10, observation.observation.revision, 2, 2)
            .unwrap();

        assert_eq!(first.data_base64, "AQI=");
        assert!(!first.complete);
        assert_eq!(second.data_base64, "AwQ=");
        assert!(second.complete);
        assert!(matches!(
            dispatcher.frame_chunk(4, 8, 10, observation.observation.revision, 0, 2),
            Err(PortalServiceError::AgentNotAttached { .. })
        ));
        assert!(matches!(
            dispatcher.frame_chunk(4, 7, 10, observation.observation.revision + 1, 0, 2),
            Err(PortalServiceError::StaleFrame { .. })
        ));
    }

    #[test]
    fn agent_coordinate_input_requires_fallback_approval() {
        let temp = TempDir::new().unwrap();
        let (mut dispatcher, executed) = dispatcher(&temp);
        let observation = dispatcher.observe(4, 7, 10).unwrap();
        let action_id = MessageId::new("coordinate-1").unwrap();
        let action = PortalAction::ClickCoordinate {
            observation_revision: observation.observation.revision,
            x: 4,
            y: 6,
        };

        let receipt = dispatcher
            .request_action_at(4, 7, action_id.clone(), 10, action.clone(), 100)
            .unwrap();
        let approval_id = match receipt.policy {
            PortalPolicyOutcome::ApprovalRequired { approval_id, .. } => approval_id,
            policy => panic!("unexpected policy outcome: {policy:?}"),
        };
        assert!(executed.lock().unwrap().is_empty());

        dispatcher
            .approve_action_at(4, &action_id, approval_id, 1_000, 101)
            .unwrap();
        assert_eq!(executed.lock().unwrap().as_slice(), &[action]);
    }

    #[test]
    fn pending_action_can_be_inspected_and_rejected_by_the_desktop() {
        let temp = TempDir::new().unwrap();
        let (mut dispatcher, executed) = dispatcher(&temp);
        let observation = dispatcher.observe(4, 7, 10).unwrap();
        let action_id = MessageId::new("reject-me").unwrap();
        let receipt = dispatcher
            .request_action_at(
                4,
                7,
                action_id.clone(),
                10,
                PortalAction::Click(
                    PortalElementRef::new(observation.observation.revision, "delete").unwrap(),
                ),
                100,
            )
            .unwrap();
        let approval_id = match receipt.policy {
            PortalPolicyOutcome::ApprovalRequired { approval_id, .. } => approval_id,
            policy => panic!("unexpected policy outcome: {policy:?}"),
        };

        let pending = dispatcher.pending_approvals(4);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].action_id, action_id);
        assert_eq!(pending[0].operation, "input");

        let rejected = dispatcher
            .reject_action(4, &action_id, approval_id, "Rejected by user")
            .unwrap();

        assert_eq!(rejected.state, PortalActionState::Failed);
        assert_eq!(
            rejected.policy,
            PortalPolicyOutcome::Denied {
                reason: "Rejected by user".to_owned()
            }
        );
        assert!(dispatcher.pending_approvals(4).is_empty());
        assert!(executed.lock().unwrap().is_empty());
    }

    #[test]
    fn conflicting_action_id_reuse_is_rejected_before_policy_evaluation() {
        let temp = TempDir::new().unwrap();
        let (mut dispatcher, _) = dispatcher(&temp);
        let observation = dispatcher.observe(4, 7, 10).unwrap();
        let action_id = MessageId::new("conflicting-action").unwrap();
        dispatcher
            .request_action_at(
                4,
                7,
                action_id.clone(),
                10,
                PortalAction::Click(
                    PortalElementRef::new(observation.observation.revision, "first").unwrap(),
                ),
                100,
            )
            .unwrap();

        let conflict = dispatcher.request_action_at(
            4,
            7,
            action_id,
            10,
            PortalAction::Click(
                PortalElementRef::new(observation.observation.revision, "second").unwrap(),
            ),
            101,
        );

        assert!(matches!(
            conflict,
            Err(PortalServiceError::ActionConflict(_))
        ));
    }

    #[test]
    fn browser_target_change_requires_a_new_input_grant() {
        let temp = TempDir::new().unwrap();
        let (mut dispatcher, _) = dispatcher(&temp);
        let observation = dispatcher.observe(4, 7, 10).unwrap();
        let click_id = MessageId::new("click-site-a").unwrap();
        let click = PortalAction::Click(
            PortalElementRef::new(observation.observation.revision, "submit").unwrap(),
        );
        let click_receipt = dispatcher
            .request_action_at(4, 7, click_id.clone(), 10, click.clone(), 100)
            .unwrap();
        let click_approval = match click_receipt.policy {
            PortalPolicyOutcome::ApprovalRequired { approval_id, .. } => approval_id,
            policy => panic!("unexpected policy outcome: {policy:?}"),
        };
        dispatcher
            .approve_action_at(4, &click_id, click_approval, 10_000, 101)
            .unwrap();

        let navigation_id = MessageId::new("navigate-site-b").unwrap();
        let navigation_receipt = dispatcher
            .request_action_at(
                4,
                7,
                navigation_id.clone(),
                10,
                PortalAction::Navigate("https://other.test".to_owned()),
                102,
            )
            .unwrap();
        let navigation_approval = match navigation_receipt.policy {
            PortalPolicyOutcome::ApprovalRequired { approval_id, .. } => approval_id,
            policy => panic!("unexpected policy outcome: {policy:?}"),
        };
        dispatcher
            .approve_action_at(4, &navigation_id, navigation_approval, 10_000, 103)
            .unwrap();

        let receipt = dispatcher
            .request_action_at(
                4,
                7,
                MessageId::new("click-site-b").unwrap(),
                10,
                click,
                104,
            )
            .unwrap();

        assert_eq!(receipt.state, PortalActionState::AwaitingApproval);
    }

    #[test]
    fn target_is_rechecked_immediately_before_dispatch() {
        let temp = TempDir::new().unwrap();
        let executed = Arc::new(Mutex::new(Vec::new()));
        let backend = FakeBackend {
            executed: Arc::clone(&executed),
            target_queries: Arc::new(Mutex::new(VecDeque::from([
                "https://example.test".to_owned(),
                "https://example.test".to_owned(),
                "https://other.test".to_owned(),
            ]))),
            ..FakeBackend::default()
        };
        let mut dispatcher = PortalDispatcher::open(temp.path()).unwrap();
        dispatcher
            .register(
                10,
                PortalScope::new(4, Some(2), 99).unwrap(),
                PortalConfig::browser("https://example.test").unwrap(),
                backend,
            )
            .unwrap();
        dispatcher.attach_agent(10, 7).unwrap();
        dispatcher.connect(10).unwrap();
        let observation = dispatcher.observe(4, 7, 10).unwrap();

        let action_id = MessageId::new("target-race").unwrap();
        let pending = dispatcher
            .request_action_at(
                4,
                7,
                action_id.clone(),
                10,
                PortalAction::Click(
                    PortalElementRef::new(observation.observation.revision, "submit").unwrap(),
                ),
                100,
            )
            .unwrap();
        let approval_id = match pending.policy {
            PortalPolicyOutcome::ApprovalRequired { approval_id, .. } => approval_id,
            policy => panic!("unexpected policy outcome: {policy:?}"),
        };
        let receipt = dispatcher
            .approve_action_at(4, &action_id, approval_id, 1_000, 101)
            .unwrap();

        assert_eq!(receipt.state, PortalActionState::Failed);
        assert_eq!(
            receipt.outcome.as_deref(),
            Some("portal target changed before dispatch")
        );
        assert!(executed.lock().unwrap().is_empty());
    }

    #[test]
    fn closing_a_portal_cancels_pending_actions_and_is_idempotent() {
        let temp = TempDir::new().unwrap();
        let (mut dispatcher, executed) = dispatcher(&temp);
        let observation = dispatcher.observe(4, 7, 10).unwrap();
        let action_id = MessageId::new("action-close").unwrap();
        dispatcher
            .request_action_at(
                4,
                7,
                action_id.clone(),
                10,
                PortalAction::Click(
                    PortalElementRef::new(observation.observation.revision, "submit").unwrap(),
                ),
                100,
            )
            .unwrap();

        dispatcher.close(10).unwrap();
        dispatcher.close(10).unwrap();

        let receipt = dispatcher.result(4, 7, &action_id).unwrap();
        assert_eq!(receipt.state, PortalActionState::Failed);
        assert_eq!(
            receipt.outcome.as_deref(),
            Some("portal closed before approval")
        );
        assert!(executed.lock().unwrap().is_empty());
    }

    #[test]
    fn disconnecting_an_agent_cancels_its_pending_action() {
        let temp = TempDir::new().unwrap();
        let (mut dispatcher, executed) = dispatcher(&temp);
        let observation = dispatcher.observe(4, 7, 10).unwrap();
        let action_id = MessageId::new("action-detach").unwrap();
        dispatcher
            .request_action_at(
                4,
                7,
                action_id.clone(),
                10,
                PortalAction::Click(
                    PortalElementRef::new(observation.observation.revision, "submit").unwrap(),
                ),
                100,
            )
            .unwrap();

        dispatcher.replace_agents(10, []).unwrap();

        let receipt = dispatcher.journal.receipt(4, &action_id).unwrap().unwrap();
        assert_eq!(receipt.state, PortalActionState::Failed);
        assert_eq!(
            receipt.outcome.as_deref(),
            Some("agent disconnected from portal before approval")
        );
        assert!(executed.lock().unwrap().is_empty());
    }

    #[test]
    fn a_closed_portal_can_reconnect_with_a_new_generation() {
        let temp = TempDir::new().unwrap();
        let (mut dispatcher, _) = dispatcher(&temp);

        dispatcher.close(10).unwrap();
        dispatcher.connect(10).unwrap();

        let portals = dispatcher.list(4, 7).unwrap();
        assert_eq!(portals.len(), 1);
        assert_eq!(portals[0].generation, 2);
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
