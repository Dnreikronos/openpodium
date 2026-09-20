use std::error::Error;
use std::fmt::{self, Display, Formatter};

use super::{Content, Handoff, HandoffId, Timestamp};

pub const MAX_HANDOFF_DEPTH: usize = 16;
const MAX_MESSAGE_ID_CHARS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HandoffMessageId(String);

impl HandoffMessageId {
    pub fn new(value: impl Into<String>) -> Result<Self, HandoffMutationError> {
        let value = value.into();
        if value.is_empty()
            || value.chars().count() > MAX_MESSAGE_ID_CHARS
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(HandoffMutationError::InvalidMessageId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for HandoffMessageId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryMechanism {
    CodexTerminal,
    ClaudeTerminal,
    OpenCodeTerminal,
}

impl Display for DeliveryMechanism {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CodexTerminal => "Codex terminal",
            Self::ClaudeTerminal => "Claude terminal",
            Self::OpenCodeTerminal => "OpenCode terminal",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryOutcome {
    Started,
    Delivered {
        finished_at: Timestamp,
    },
    Failed {
        finished_at: Timestamp,
        error: Content,
        retryable: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryAttempt {
    ordinal: u32,
    message_id: HandoffMessageId,
    started_at: Timestamp,
    mechanism: DeliveryMechanism,
    outcome: DeliveryOutcome,
}

impl DeliveryAttempt {
    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    pub const fn message_id(&self) -> &HandoffMessageId {
        &self.message_id
    }

    pub const fn started_at(&self) -> Timestamp {
        self.started_at
    }

    pub const fn mechanism(&self) -> DeliveryMechanism {
        self.mechanism
    }

    pub const fn outcome(&self) -> &DeliveryOutcome {
        &self.outcome
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffProgress {
    message_id: HandoffMessageId,
    body: Content,
    reported_at: Timestamp,
}

impl HandoffProgress {
    pub const fn new(message_id: HandoffMessageId, body: Content, reported_at: Timestamp) -> Self {
        Self {
            message_id,
            body,
            reported_at,
        }
    }

    pub const fn message_id(&self) -> &HandoffMessageId {
        &self.message_id
    }

    pub const fn body(&self) -> &Content {
        &self.body
    }

    pub const fn reported_at(&self) -> Timestamp {
        self.reported_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffResponseStatus {
    Completed,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffResponse {
    message_id: HandoffMessageId,
    status: HandoffResponseStatus,
    body: Content,
    responded_at: Timestamp,
}

impl HandoffResponse {
    pub const fn new(
        message_id: HandoffMessageId,
        status: HandoffResponseStatus,
        body: Content,
        responded_at: Timestamp,
    ) -> Self {
        Self {
            message_id,
            status,
            body,
            responded_at,
        }
    }

    pub const fn message_id(&self) -> &HandoffMessageId {
        &self.message_id
    }

    pub const fn status(&self) -> HandoffResponseStatus {
        self.status
    }

    pub const fn body(&self) -> &Content {
        &self.body
    }

    pub const fn responded_at(&self) -> Timestamp {
        self.responded_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoffTermination {
    Cancelled {
        message_id: HandoffMessageId,
        reason: Content,
        cancelled_at: Timestamp,
    },
    TimedOut {
        timed_out_at: Timestamp,
    },
}

impl Handoff {
    #[allow(clippy::too_many_arguments)]
    pub fn tracked(
        id: HandoffId,
        message_id: HandoffMessageId,
        origin: super::HandoffOrigin,
        recipient: super::AgentId,
        payload: super::HandoffPayload,
        parent: Option<HandoffId>,
        created_at: Timestamp,
        response_deadline: Option<Timestamp>,
    ) -> Result<Self, HandoffMutationError> {
        if response_deadline.is_some_and(|deadline| deadline <= created_at) {
            return Err(HandoffMutationError::InvalidResponseDeadline);
        }
        Ok(Self {
            id,
            origin,
            recipient,
            payload,
            message_id: Some(message_id),
            parent,
            created_at: Some(created_at),
            response_deadline,
            delivery_attempts: Vec::new(),
            progress: Vec::new(),
            response: None,
            termination: None,
        })
    }

    pub const fn message_id(&self) -> Option<&HandoffMessageId> {
        self.message_id.as_ref()
    }

    pub const fn parent(&self) -> Option<HandoffId> {
        self.parent
    }

    pub const fn created_at(&self) -> Option<Timestamp> {
        self.created_at
    }

    pub const fn response_deadline(&self) -> Option<Timestamp> {
        self.response_deadline
    }

    pub fn delivery_attempts(&self) -> &[DeliveryAttempt] {
        &self.delivery_attempts
    }

    pub fn progress(&self) -> &[HandoffProgress] {
        &self.progress
    }

    pub const fn response(&self) -> Option<&HandoffResponse> {
        self.response.as_ref()
    }

    pub const fn termination(&self) -> Option<&HandoffTermination> {
        self.termination.as_ref()
    }

    pub fn is_delivered(&self) -> bool {
        self.message_id
            .as_ref()
            .is_some_and(|message_id| self.is_message_delivered(message_id))
    }

    pub fn is_message_delivered(&self, message_id: &HandoffMessageId) -> bool {
        self.delivery_attempts.iter().any(|attempt| {
            &attempt.message_id == message_id
                && matches!(attempt.outcome(), DeliveryOutcome::Delivered { .. })
        })
    }

    pub fn is_delivery_terminal(&self, message_id: &HandoffMessageId) -> bool {
        self.delivery_attempts.iter().any(|attempt| {
            &attempt.message_id == message_id
                && matches!(
                    attempt.outcome(),
                    DeliveryOutcome::Delivered { .. }
                        | DeliveryOutcome::Failed {
                            retryable: false,
                            ..
                        }
                )
        })
    }

    pub fn begin_delivery(
        &mut self,
        message_id: HandoffMessageId,
        mechanism: DeliveryMechanism,
        started_at: Timestamp,
    ) -> Result<u32, HandoffMutationError> {
        if !self.has_delivery_message(&message_id) {
            return Err(HandoffMutationError::UnknownDeliveryMessage);
        }
        if self.message_id.as_ref() == Some(&message_id)
            && (self.response.is_some() || self.termination.is_some())
        {
            return Err(HandoffMutationError::AlreadyTerminal);
        }
        if self.is_message_delivered(&message_id) {
            return Err(HandoffMutationError::AlreadyDelivered);
        }
        if self
            .delivery_attempts
            .iter()
            .any(|attempt| matches!(attempt.outcome, DeliveryOutcome::Started))
        {
            return Err(HandoffMutationError::AttemptInProgress);
        }
        let ordinal = u32::try_from(self.delivery_attempts.len())
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(HandoffMutationError::AttemptLimitReached)?;
        self.delivery_attempts.push(DeliveryAttempt {
            ordinal,
            message_id,
            started_at,
            mechanism,
            outcome: DeliveryOutcome::Started,
        });
        Ok(ordinal)
    }

    pub fn complete_delivery(
        &mut self,
        ordinal: u32,
        finished_at: Timestamp,
    ) -> Result<(), HandoffMutationError> {
        let attempt = self.started_attempt(ordinal, finished_at)?;
        attempt.outcome = DeliveryOutcome::Delivered { finished_at };
        Ok(())
    }

    pub fn fail_delivery(
        &mut self,
        ordinal: u32,
        finished_at: Timestamp,
        error: Content,
        retryable: bool,
    ) -> Result<(), HandoffMutationError> {
        let attempt = self.started_attempt(ordinal, finished_at)?;
        attempt.outcome = DeliveryOutcome::Failed {
            finished_at,
            error,
            retryable,
        };
        Ok(())
    }

    pub fn report_progress(
        &mut self,
        progress: HandoffProgress,
    ) -> Result<(), HandoffMutationError> {
        self.ensure_delivered_and_active()?;
        if self
            .progress
            .iter()
            .any(|entry| entry.message_id == progress.message_id)
        {
            return Err(HandoffMutationError::DuplicateMessageId);
        }
        self.progress.push(progress);
        Ok(())
    }

    pub fn respond(&mut self, response: HandoffResponse) -> Result<(), HandoffMutationError> {
        self.ensure_delivered_and_active()?;
        self.response = Some(response);
        Ok(())
    }

    pub fn cancel(
        &mut self,
        message_id: HandoffMessageId,
        reason: Content,
        cancelled_at: Timestamp,
    ) -> Result<(), HandoffMutationError> {
        self.ensure_active()?;
        self.termination = Some(HandoffTermination::Cancelled {
            message_id,
            reason,
            cancelled_at,
        });
        Ok(())
    }

    pub fn time_out(&mut self, timed_out_at: Timestamp) -> Result<(), HandoffMutationError> {
        self.ensure_active()?;
        let deadline = self
            .response_deadline
            .ok_or(HandoffMutationError::NoResponseDeadline)?;
        if timed_out_at < deadline {
            return Err(HandoffMutationError::ResponseDeadlineNotReached);
        }
        self.termination = Some(HandoffTermination::TimedOut { timed_out_at });
        Ok(())
    }

    pub(crate) fn validate_successor(&self, previous: &Self) -> Result<(), HandoffMutationError> {
        if self.id != previous.id
            || self.origin != previous.origin
            || self.recipient != previous.recipient
            || self.payload != previous.payload
            || self.message_id != previous.message_id
            || self.parent != previous.parent
            || self.created_at != previous.created_at
            || self.response_deadline != previous.response_deadline
        {
            return Err(HandoffMutationError::IdentityChanged);
        }

        let attempts_changed = self.delivery_attempts != previous.delivery_attempts;
        let progress_changed = self.progress != previous.progress;
        let response_changed = self.response != previous.response;
        let termination_changed = self.termination != previous.termination;
        if [
            attempts_changed,
            progress_changed,
            response_changed,
            termination_changed,
        ]
        .into_iter()
        .filter(|changed| *changed)
        .count()
            != 1
        {
            return Err(HandoffMutationError::NonAtomicChange);
        }

        let mut candidate = previous.clone();
        if attempts_changed {
            if self.delivery_attempts.len() == previous.delivery_attempts.len() + 1 {
                let attempt = self
                    .delivery_attempts
                    .last()
                    .expect("one delivery attempt was appended");
                if !matches!(attempt.outcome, DeliveryOutcome::Started) {
                    return Err(HandoffMutationError::NonAtomicChange);
                }
                let ordinal = candidate.begin_delivery(
                    attempt.message_id.clone(),
                    attempt.mechanism,
                    attempt.started_at,
                )?;
                if ordinal != attempt.ordinal {
                    return Err(HandoffMutationError::NonAtomicChange);
                }
            } else if self.delivery_attempts.len() == previous.delivery_attempts.len() {
                let mut changed = self
                    .delivery_attempts
                    .iter()
                    .zip(&previous.delivery_attempts)
                    .filter(|(after, before)| after != before);
                let Some((attempt, _)) = changed.next() else {
                    return Err(HandoffMutationError::NonAtomicChange);
                };
                if changed.next().is_some() {
                    return Err(HandoffMutationError::NonAtomicChange);
                }
                match &attempt.outcome {
                    DeliveryOutcome::Delivered { finished_at } => {
                        candidate.complete_delivery(attempt.ordinal, *finished_at)?;
                    }
                    DeliveryOutcome::Failed {
                        finished_at,
                        error,
                        retryable,
                    } => {
                        candidate.fail_delivery(
                            attempt.ordinal,
                            *finished_at,
                            error.clone(),
                            *retryable,
                        )?;
                    }
                    DeliveryOutcome::Started => {
                        return Err(HandoffMutationError::NonAtomicChange);
                    }
                }
            } else {
                return Err(HandoffMutationError::NonAtomicChange);
            }
        } else if progress_changed {
            if self.progress.len() != previous.progress.len() + 1
                || self.progress[..previous.progress.len()] != previous.progress
            {
                return Err(HandoffMutationError::NonAtomicChange);
            }
            candidate.report_progress(
                self.progress
                    .last()
                    .expect("one progress entry was appended")
                    .clone(),
            )?;
        } else if response_changed {
            if previous.response.is_some() {
                return Err(HandoffMutationError::NonAtomicChange);
            }
            candidate.respond(
                self.response
                    .as_ref()
                    .expect("response changed from none to some")
                    .clone(),
            )?;
        } else if termination_changed {
            if previous.termination.is_some() {
                return Err(HandoffMutationError::NonAtomicChange);
            }
            match self
                .termination
                .as_ref()
                .expect("termination changed from none to some")
            {
                HandoffTermination::Cancelled {
                    message_id,
                    reason,
                    cancelled_at,
                } => candidate.cancel(message_id.clone(), reason.clone(), *cancelled_at)?,
                HandoffTermination::TimedOut { timed_out_at } => {
                    candidate.time_out(*timed_out_at)?;
                }
            }
        }

        if &candidate != self {
            return Err(HandoffMutationError::NonAtomicChange);
        }
        Ok(())
    }

    fn ensure_active(&self) -> Result<(), HandoffMutationError> {
        if self.response.is_some() || self.termination.is_some() {
            return Err(HandoffMutationError::AlreadyTerminal);
        }
        if self.message_id.is_none() {
            return Err(HandoffMutationError::LegacyHandoff);
        }
        Ok(())
    }

    fn has_delivery_message(&self, message_id: &HandoffMessageId) -> bool {
        self.message_id.as_ref() == Some(message_id)
            || self
                .progress
                .iter()
                .any(|entry| entry.message_id() == message_id)
            || self
                .response
                .as_ref()
                .is_some_and(|response| response.message_id() == message_id)
            || matches!(
                &self.termination,
                Some(HandoffTermination::Cancelled {
                    message_id: cancellation_id,
                    ..
                }) if cancellation_id == message_id
            )
    }

    fn ensure_delivered_and_active(&self) -> Result<(), HandoffMutationError> {
        self.ensure_active()?;
        if !self.is_delivered() {
            return Err(HandoffMutationError::NotDelivered);
        }
        Ok(())
    }

    fn started_attempt(
        &mut self,
        ordinal: u32,
        finished_at: Timestamp,
    ) -> Result<&mut DeliveryAttempt, HandoffMutationError> {
        let attempt = self
            .delivery_attempts
            .iter_mut()
            .find(|attempt| attempt.ordinal == ordinal)
            .ok_or(HandoffMutationError::UnknownAttempt)?;
        if !matches!(attempt.outcome, DeliveryOutcome::Started) {
            return Err(HandoffMutationError::AttemptAlreadyFinished);
        }
        if finished_at < attempt.started_at {
            return Err(HandoffMutationError::InvalidAttemptTime);
        }
        Ok(attempt)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffMutationError {
    InvalidMessageId,
    InvalidResponseDeadline,
    DuplicateMessageId,
    UnknownDeliveryMessage,
    AlreadyDelivered,
    AttemptInProgress,
    AttemptLimitReached,
    UnknownAttempt,
    AttemptAlreadyFinished,
    InvalidAttemptTime,
    NotDelivered,
    AlreadyTerminal,
    NoResponseDeadline,
    ResponseDeadlineNotReached,
    LegacyHandoff,
    IdentityChanged,
    NonAtomicChange,
}

impl Display for HandoffMutationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidMessageId => "handoff message ID is invalid",
            Self::InvalidResponseDeadline => "response deadline must be after creation",
            Self::DuplicateMessageId => "handoff message ID was already recorded",
            Self::UnknownDeliveryMessage => "delivery message is not part of this handoff",
            Self::AlreadyDelivered => "handoff was already delivered",
            Self::AttemptInProgress => "handoff already has a delivery attempt in progress",
            Self::AttemptLimitReached => "delivery attempt limit was reached",
            Self::UnknownAttempt => "delivery attempt does not exist",
            Self::AttemptAlreadyFinished => "delivery attempt is already finished",
            Self::InvalidAttemptTime => "delivery completion precedes its start",
            Self::NotDelivered => "handoff has not been delivered",
            Self::AlreadyTerminal => "handoff is already terminal",
            Self::NoResponseDeadline => "handoff has no response deadline",
            Self::ResponseDeadlineNotReached => "handoff response deadline has not been reached",
            Self::LegacyHandoff => "legacy handoff does not support orchestration transitions",
            Self::IdentityChanged => "handoff identity cannot change",
            Self::NonAtomicChange => "handoff update must contain one atomic transition",
        })
    }
}

impl Error for HandoffMutationError {}
