use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use super::{CapabilityStatus, PortalCapabilities, PortalOperation};

pub const DEFAULT_GRANT_LIFETIME_MS: u64 = 5 * 60 * 1_000;

const MAX_TARGET_KEY_CHARS: usize = 2_048;
const PENDING_APPROVAL_LIFETIME_MS: u64 = 60 * 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRequest {
    agent_id: u64,
    portal_id: u64,
    operation: PortalOperation,
    target: String,
    capabilities: PortalCapabilities,
}

impl PolicyRequest {
    pub fn new(
        agent_id: u64,
        portal_id: u64,
        operation: PortalOperation,
        target: impl Into<String>,
        capabilities: PortalCapabilities,
    ) -> Result<Self, PortalPolicyError> {
        if agent_id == 0 || portal_id == 0 {
            return Err(PortalPolicyError::InvalidIdentity);
        }
        let target = target.into();
        if target.trim().is_empty() {
            return Err(PortalPolicyError::InvalidTarget);
        }
        if target.chars().count() > MAX_TARGET_KEY_CHARS || target.contains('\0') {
            return Err(PortalPolicyError::InvalidTarget);
        }
        Ok(Self {
            agent_id,
            portal_id,
            operation,
            target,
            capabilities,
        })
    }

    pub const fn agent_id(&self) -> u64 {
        self.agent_id
    }

    pub const fn portal_id(&self) -> u64 {
        self.portal_id
    }

    pub const fn operation(&self) -> PortalOperation {
        self.operation
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    pub const fn capabilities(&self) -> &PortalCapabilities {
        &self.capabilities
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyRule {
    Allow,
    RequireApproval { reason: String },
    Deny { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allowed { grant_id: Option<u64> },
    ApprovalRequired { approval_id: u64, reason: String },
    Denied { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingApproval {
    id: u64,
    request: PolicyRequest,
    requested_at_ms: u64,
}

impl PendingApproval {
    pub const fn id(&self) -> u64 {
        self.id
    }

    pub const fn request(&self) -> &PolicyRequest {
        &self.request
    }

    pub const fn requested_at_ms(&self) -> u64 {
        self.requested_at_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalGrant {
    id: u64,
    agent_id: u64,
    portal_id: u64,
    operation: PortalOperation,
    target: String,
    expires_at_ms: u64,
}

impl PortalGrant {
    pub const fn id(&self) -> u64 {
        self.id
    }

    pub const fn agent_id(&self) -> u64 {
        self.agent_id
    }

    pub const fn portal_id(&self) -> u64 {
        self.portal_id
    }

    pub const fn operation(&self) -> PortalOperation {
        self.operation
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    pub const fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }
}

#[derive(Debug, Default)]
pub struct PortalPolicy {
    rules: BTreeMap<PortalOperation, PolicyRule>,
    grants: BTreeMap<u64, PortalGrant>,
    pending: BTreeMap<u64, PendingApproval>,
    next_id: u64,
}

impl PortalPolicy {
    pub fn new() -> Self {
        let mut policy = Self {
            next_id: 1,
            ..Self::default()
        };
        policy
            .rules
            .insert(PortalOperation::Observe, PolicyRule::Allow);
        policy
            .rules
            .insert(PortalOperation::Screenshot, PolicyRule::Allow);
        for operation in [
            PortalOperation::Navigate,
            PortalOperation::Input,
            PortalOperation::CoordinateFallback,
            PortalOperation::Upload,
            PortalOperation::Download,
            PortalOperation::Clipboard,
            PortalOperation::SensitivePermission,
        ] {
            policy.rules.insert(
                operation,
                PolicyRule::RequireApproval {
                    reason: "the operation may have consequences outside the canvas".to_owned(),
                },
            );
        }
        policy
    }

    pub fn set_rule(&mut self, operation: PortalOperation, rule: PolicyRule) {
        if matches!(rule, PolicyRule::Deny { .. }) {
            self.grants.retain(|_, grant| grant.operation != operation);
        }
        self.rules.insert(operation, rule);
    }

    pub fn evaluate(&mut self, request: &PolicyRequest, now_ms: u64) -> PolicyDecision {
        self.grants.retain(|_, grant| grant.expires_at_ms > now_ms);
        // An expired approval can no longer be granted, so dropping it here
        // stops a later request from reusing an ID that only returns
        // `StaleApproval`.
        self.pending.retain(|_, approval| {
            now_ms.saturating_sub(approval.requested_at_ms) <= PENDING_APPROVAL_LIFETIME_MS
        });
        if let Some(CapabilityStatus::Unavailable { reason }) = capability_status(request) {
            return PolicyDecision::Denied {
                reason: reason.clone(),
            };
        }
        if let Some(PolicyRule::Deny { reason }) = self.rules.get(&request.operation) {
            return PolicyDecision::Denied {
                reason: reason.clone(),
            };
        }
        if let Some(grant) = self
            .grants
            .values()
            .find(|grant| matches_request(grant, request))
        {
            return PolicyDecision::Allowed {
                grant_id: Some(grant.id),
            };
        }

        if let Some(decision) = capability_decision(request) {
            return self.pending_decision(request, now_ms, decision);
        }
        match self.rules.get(&request.operation) {
            Some(PolicyRule::Allow) => PolicyDecision::Allowed { grant_id: None },
            Some(PolicyRule::RequireApproval { reason }) => {
                self.pending_decision(request, now_ms, reason.clone())
            }
            Some(PolicyRule::Deny { reason }) => PolicyDecision::Denied {
                reason: reason.clone(),
            },
            None => self.pending_decision(
                request,
                now_ms,
                "no policy rule exists for this operation".to_owned(),
            ),
        }
    }

    pub fn approve(
        &mut self,
        approval_id: u64,
        request: &PolicyRequest,
        now_ms: u64,
        lifetime_ms: u64,
    ) -> Result<PortalGrant, PortalPolicyError> {
        let pending = self
            .pending
            .get(&approval_id)
            .ok_or(PortalPolicyError::UnknownApproval(approval_id))?;
        if pending.request != *request
            || now_ms.saturating_sub(pending.requested_at_ms) > PENDING_APPROVAL_LIFETIME_MS
        {
            self.pending.remove(&approval_id);
            return Err(PortalPolicyError::StaleApproval);
        }
        if let Some(CapabilityStatus::Unavailable { reason }) = capability_status(request) {
            let reason = reason.clone();
            self.pending.remove(&approval_id);
            return Err(PortalPolicyError::Unavailable(reason));
        }
        if let Some(PolicyRule::Deny { reason }) = self.rules.get(&request.operation) {
            let reason = reason.clone();
            self.pending.remove(&approval_id);
            return Err(PortalPolicyError::Denied(reason));
        }
        // A bad lifetime is the caller's mistake, so the approval stays
        // usable rather than making the desktop prompt unanswerable.
        if lifetime_ms == 0 {
            return Err(PortalPolicyError::InvalidLifetime);
        }
        self.pending.remove(&approval_id);

        let grant = PortalGrant {
            id: self.allocate_id(),
            agent_id: request.agent_id,
            portal_id: request.portal_id,
            operation: request.operation,
            target: request.target.clone(),
            expires_at_ms: now_ms.saturating_add(lifetime_ms),
        };
        self.grants.insert(grant.id, grant.clone());
        Ok(grant)
    }

    pub fn revoke_grant(&mut self, grant_id: u64) -> bool {
        self.grants.remove(&grant_id).is_some()
    }

    pub fn reject(&mut self, approval_id: u64) -> Result<(), PortalPolicyError> {
        self.pending
            .remove(&approval_id)
            .map(|_| ())
            .ok_or(PortalPolicyError::UnknownApproval(approval_id))
    }

    pub fn revoke_agent(&mut self, agent_id: u64) {
        self.grants.retain(|_, grant| grant.agent_id != agent_id);
        self.pending
            .retain(|_, approval| approval.request.agent_id != agent_id);
    }

    pub fn revoke_connection(&mut self, agent_id: u64, portal_id: u64) {
        self.grants
            .retain(|_, grant| grant.agent_id != agent_id || grant.portal_id != portal_id);
        self.pending.retain(|_, approval| {
            approval.request.agent_id != agent_id || approval.request.portal_id != portal_id
        });
    }

    pub fn revoke_portal(&mut self, portal_id: u64) {
        self.grants.retain(|_, grant| grant.portal_id != portal_id);
        self.pending
            .retain(|_, approval| approval.request.portal_id != portal_id);
    }

    pub fn revoke_portal_grants(&mut self, portal_id: u64) {
        self.grants.retain(|_, grant| grant.portal_id != portal_id);
    }

    pub fn pending_approval(&self, approval_id: u64) -> Option<&PendingApproval> {
        self.pending.get(&approval_id)
    }

    fn pending_decision(
        &mut self,
        request: &PolicyRequest,
        now_ms: u64,
        reason: String,
    ) -> PolicyDecision {
        // Every request gets its own approval. Sharing one across identical
        // requests let the first resolution consume it and strand the rest.
        let id = self.allocate_id();
        self.pending.insert(
            id,
            PendingApproval {
                id,
                request: request.clone(),
                requested_at_ms: now_ms,
            },
        );
        PolicyDecision::ApprovalRequired {
            approval_id: id,
            reason,
        }
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).unwrap_or(1);
        id
    }
}

fn capability_status(request: &PolicyRequest) -> Option<&CapabilityStatus> {
    match request.capabilities.status(request.operation) {
        CapabilityStatus::Supported => None,
        status => Some(status),
    }
}

fn capability_decision(request: &PolicyRequest) -> Option<String> {
    match capability_status(request) {
        Some(CapabilityStatus::Unavailable { reason }) => Some(reason.clone()),
        Some(CapabilityStatus::PermissionRequired { reason }) => Some(reason.clone()),
        Some(CapabilityStatus::Supported) | None => None,
    }
}

fn matches_request(grant: &PortalGrant, request: &PolicyRequest) -> bool {
    grant.agent_id == request.agent_id
        && grant.portal_id == request.portal_id
        && grant.operation == request.operation
        && grant.target == request.target
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalPolicyError {
    InvalidIdentity,
    InvalidTarget,
    UnknownApproval(u64),
    StaleApproval,
    Unavailable(String),
    Denied(String),
    InvalidLifetime,
}

impl Display for PortalPolicyError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentity => formatter.write_str("agent and portal IDs must be positive"),
            Self::InvalidTarget => formatter.write_str("portal policy target is invalid"),
            Self::UnknownApproval(id) => write!(formatter, "approval {id} does not exist"),
            Self::StaleApproval => {
                formatter.write_str("portal approval no longer matches the request")
            }
            Self::Unavailable(reason) => {
                write!(formatter, "portal operation is unavailable: {reason}")
            }
            Self::Denied(reason) => write!(formatter, "portal operation is denied: {reason}"),
            Self::InvalidLifetime => {
                formatter.write_str("portal grant lifetime must be greater than zero")
            }
        }
    }
}

impl Error for PortalPolicyError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal::{CapabilityStatus, PortalCapabilities};

    fn request(operation: PortalOperation, target: &str) -> PolicyRequest {
        PolicyRequest::new(
            7,
            11,
            operation,
            target,
            PortalCapabilities::browser_defaults(),
        )
        .unwrap()
    }

    #[test]
    fn observation_is_allowed_but_consequential_input_requires_approval() {
        let mut policy = PortalPolicy::new();
        assert_eq!(
            policy.evaluate(
                &request(PortalOperation::Observe, "https://example.test"),
                10
            ),
            PolicyDecision::Allowed { grant_id: None }
        );
        assert!(matches!(
            policy.evaluate(&request(PortalOperation::Input, "https://example.test"), 10),
            PolicyDecision::ApprovalRequired { .. }
        ));
    }

    #[test]
    fn approval_is_bound_to_target_and_rechecked_before_granting() {
        let mut policy = PortalPolicy::new();
        let original = request(PortalOperation::Input, "https://example.test");
        let approval_id = match policy.evaluate(&original, 10) {
            PolicyDecision::ApprovalRequired { approval_id, .. } => approval_id,
            decision => panic!("unexpected decision: {decision:?}"),
        };
        let changed = request(PortalOperation::Input, "https://other.test");
        assert_eq!(
            policy.approve(approval_id, &changed, 11, 1_000),
            Err(PortalPolicyError::StaleApproval)
        );
        let approval_id = match policy.evaluate(&original, 12) {
            PolicyDecision::ApprovalRequired { approval_id, .. } => approval_id,
            decision => panic!("unexpected decision: {decision:?}"),
        };
        let grant = policy.approve(approval_id, &original, 13, 1_000).unwrap();
        assert!(matches!(
            policy.evaluate(&original, 14),
            PolicyDecision::Allowed { grant_id: Some(id) } if id == grant.id()
        ));
        assert!(matches!(
            policy.evaluate(&original, 1_014),
            PolicyDecision::ApprovalRequired { .. }
        ));
    }

    #[test]
    fn identical_requests_get_their_own_approvals() {
        let mut policy = PortalPolicy::new();
        let request = request(PortalOperation::Input, "https://example.test");
        let first = match policy.evaluate(&request, 10) {
            PolicyDecision::ApprovalRequired { approval_id, .. } => approval_id,
            decision => panic!("unexpected decision: {decision:?}"),
        };
        let second = match policy.evaluate(&request, 10) {
            PolicyDecision::ApprovalRequired { approval_id, .. } => approval_id,
            decision => panic!("unexpected decision: {decision:?}"),
        };

        assert_ne!(first, second);
        assert!(policy.approve(first, &request, 11, 1_000).is_ok());
        // Resolving one must not consume the other.
        assert!(policy.reject(second).is_ok());
    }

    #[test]
    fn a_rejected_lifetime_leaves_the_approval_answerable() {
        let mut policy = PortalPolicy::new();
        let request = request(PortalOperation::Input, "https://example.test");
        let approval_id = match policy.evaluate(&request, 10) {
            PolicyDecision::ApprovalRequired { approval_id, .. } => approval_id,
            decision => panic!("unexpected decision: {decision:?}"),
        };

        assert_eq!(
            policy.approve(approval_id, &request, 11, 0),
            Err(PortalPolicyError::InvalidLifetime)
        );
        assert!(policy.approve(approval_id, &request, 12, 1_000).is_ok());
    }

    #[test]
    fn an_expired_approval_is_replaced_rather_than_reissued() {
        let mut policy = PortalPolicy::new();
        let request = request(PortalOperation::Input, "https://example.test");
        let expired = match policy.evaluate(&request, 10) {
            PolicyDecision::ApprovalRequired { approval_id, .. } => approval_id,
            decision => panic!("unexpected decision: {decision:?}"),
        };
        let later = 10 + PENDING_APPROVAL_LIFETIME_MS + 1;

        let reissued = match policy.evaluate(&request, later) {
            PolicyDecision::ApprovalRequired { approval_id, .. } => approval_id,
            decision => panic!("unexpected decision: {decision:?}"),
        };
        assert_ne!(reissued, expired);
        assert_eq!(
            policy.approve(expired, &request, later, 1_000),
            Err(PortalPolicyError::UnknownApproval(expired))
        );
        assert!(policy.approve(reissued, &request, later, 1_000).is_ok());
    }

    #[test]
    fn unavailable_capabilities_cannot_be_approved() {
        let mut policy = PortalPolicy::new();
        let mut capabilities = PortalCapabilities::browser_defaults();
        capabilities.set_status(
            PortalOperation::Input,
            CapabilityStatus::Unavailable {
                reason: "driver missing".to_owned(),
            },
        );
        let request = PolicyRequest::new(
            7,
            11,
            PortalOperation::Input,
            "device:emulator",
            capabilities,
        )
        .unwrap();
        assert!(matches!(
            policy.evaluate(&request, 0),
            PolicyDecision::Denied { reason } if reason == "driver missing"
        ));
    }

    #[test]
    fn explicit_denial_invalidates_existing_grants() {
        let mut policy = PortalPolicy::new();
        let request = request(PortalOperation::Input, "https://example.test");
        let approval_id = match policy.evaluate(&request, 0) {
            PolicyDecision::ApprovalRequired { approval_id, .. } => approval_id,
            decision => panic!("unexpected decision: {decision:?}"),
        };
        policy.approve(approval_id, &request, 1, 1_000).unwrap();

        policy.set_rule(
            PortalOperation::Input,
            PolicyRule::Deny {
                reason: "input disabled".to_owned(),
            },
        );

        assert_eq!(
            policy.evaluate(&request, 2),
            PolicyDecision::Denied {
                reason: "input disabled".to_owned(),
            }
        );
    }

    #[test]
    fn revocation_removes_grants_for_an_agent_or_portal() {
        let mut policy = PortalPolicy::new();
        let request = request(PortalOperation::Input, "https://example.test");
        let approval_id = match policy.evaluate(&request, 0) {
            PolicyDecision::ApprovalRequired { approval_id, .. } => approval_id,
            decision => panic!("unexpected decision: {decision:?}"),
        };
        let grant = policy.approve(approval_id, &request, 1, 1_000).unwrap();
        assert!(policy.revoke_grant(grant.id()));
        assert!(!policy.revoke_grant(grant.id()));
    }
}
