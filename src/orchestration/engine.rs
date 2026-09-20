use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use crate::domain::{
    AgentId, AgentProgram, Content, DeliveryMechanism, DeliveryOutcome, DomainCommand, Handoff,
    HandoffId, HandoffMessageId, HandoffPayload, HandoffProgress, HandoffResponse,
    HandoffResponseStatus, HandoffTermination, Name, Task, TaskId, TaskState, Timestamp, Workspace,
    WorkspaceId,
};
use crate::ipc::{AcceptedMessage, HandoffKind, MessageId, ProtocolCommand, ResponseStatus};
use crate::persistence::PersistenceError;
use crate::workspaces::{WorkspaceError, WorkspaceManager};

use super::prompt;

const DELIVERY_WINDOW_MS: u64 = 30_000;
const INITIAL_RETRY_MS: u64 = 250;
const MAX_BACKOFF_SHIFT: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct MailboxKey {
    workspace_id: WorkspaceId,
    recipient_agent_id: AgentId,
}

#[derive(Debug, Clone)]
struct PendingDelivery {
    workspace_id: WorkspaceId,
    handoff_id: HandoffId,
    message_id: HandoffMessageId,
    recipient_agent_id: AgentId,
    mechanism: DeliveryMechanism,
    prompt: String,
    deadline: Timestamp,
    next_attempt_at: Timestamp,
    failures: u32,
}

#[derive(Debug, Clone)]
pub struct DeliveryRequest {
    pending: PendingDelivery,
    attempt_ordinal: u32,
}

impl DeliveryRequest {
    pub const fn workspace_id(&self) -> WorkspaceId {
        self.pending.workspace_id
    }

    pub const fn handoff_id(&self) -> HandoffId {
        self.pending.handoff_id
    }

    pub const fn recipient_agent_id(&self) -> AgentId {
        self.pending.recipient_agent_id
    }

    pub const fn mechanism(&self) -> DeliveryMechanism {
        self.pending.mechanism
    }

    pub fn prompt(&self) -> &str {
        &self.pending.prompt
    }
}

#[derive(Default)]
pub struct Orchestrator {
    mailboxes: BTreeMap<MailboxKey, VecDeque<PendingDelivery>>,
}

impl Orchestrator {
    pub fn recover(
        workspaces: &mut WorkspaceManager,
        recovered_at: Timestamp,
    ) -> Result<Self, OrchestrationError> {
        let interrupted: Vec<_> = workspaces
            .recent_workspaces()
            .flat_map(|workspace| {
                workspace.handoffs().flat_map(move |handoff| {
                    handoff
                        .delivery_attempts()
                        .iter()
                        .filter_map(move |attempt| {
                            matches!(attempt.outcome(), DeliveryOutcome::Started).then_some((
                                workspace.id(),
                                handoff.id(),
                                attempt.ordinal(),
                            ))
                        })
                })
            })
            .collect();
        for (workspace_id, handoff_id, ordinal) in interrupted {
            let before = handoff(workspaces, workspace_id, handoff_id)?.clone();
            let mut after = before.clone();
            after.fail_delivery(
                ordinal,
                recovered_at,
                content("delivery outcome is unknown after OpenPodium restarted")?,
                true,
            )?;
            execute_handoff_update(workspaces, workspace_id, before, after, recovered_at)?;
        }

        let mut orchestrator = Self::default();
        let candidates: Vec<_> = workspaces
            .recent_workspaces()
            .flat_map(|workspace| {
                workspace.handoffs().flat_map(move |handoff| {
                    delivery_message_ids(handoff)
                        .into_iter()
                        .map(move |message_id| (workspace.id(), handoff.id(), message_id))
                })
            })
            .collect();
        for (workspace_id, handoff_id, message_id) in candidates {
            orchestrator.enqueue(
                workspaces,
                workspace_id,
                handoff_id,
                &message_id,
                recovered_at,
            )?;
        }
        orchestrator.expire_due(workspaces, recovered_at)?;
        Ok(orchestrator)
    }

    pub fn accept(
        &mut self,
        workspaces: &mut WorkspaceManager,
        message: &AcceptedMessage,
        accepted_at: Timestamp,
    ) -> Result<(), OrchestrationError> {
        let workspace_id = WorkspaceId::new(message.workspace_id);
        match &message.command {
            ProtocolCommand::SendTask {
                message_id,
                recipient_agent_id,
                title,
                body,
            } => self.accept_handoff(
                workspaces,
                workspace_id,
                AgentId::new(message.sender_agent_id),
                AgentId::new(*recipient_agent_id),
                message_id,
                HandoffKind::Task,
                Some(title),
                body,
                None,
                None,
                accepted_at,
            ),
            ProtocolCommand::SendHandoff {
                message_id,
                recipient_agent_id,
                kind,
                title,
                body,
                parent_message_id,
                response_timeout_ms,
            } => self.accept_handoff(
                workspaces,
                workspace_id,
                AgentId::new(message.sender_agent_id),
                AgentId::new(*recipient_agent_id),
                message_id,
                *kind,
                title.as_ref(),
                body,
                parent_message_id.as_ref(),
                *response_timeout_ms,
                accepted_at,
            ),
            ProtocolCommand::ReportProgress {
                message_id,
                task_message_id,
                body,
            } => self.accept_progress(
                workspaces,
                message,
                message_id,
                task_message_id,
                body,
                accepted_at,
            ),
            ProtocolCommand::ReportHandoffProgress {
                message_id,
                handoff_message_id,
                body,
            } => self.accept_progress(
                workspaces,
                message,
                message_id,
                handoff_message_id,
                body,
                accepted_at,
            ),
            ProtocolCommand::Respond {
                message_id,
                task_message_id,
                status,
                body,
            } => self.accept_response(
                workspaces,
                workspace_id,
                message,
                message_id,
                task_message_id,
                *status,
                body,
                accepted_at,
            ),
            ProtocolCommand::RespondToHandoff {
                message_id,
                handoff_message_id,
                status,
                body,
            } => self.accept_response(
                workspaces,
                workspace_id,
                message,
                message_id,
                handoff_message_id,
                *status,
                body,
                accepted_at,
            ),
            ProtocolCommand::CancelHandoff {
                message_id,
                handoff_message_id,
                reason,
            } => self.accept_cancellation(
                workspaces,
                workspace_id,
                message,
                message_id,
                handoff_message_id,
                reason,
                accepted_at,
            ),
            ProtocolCommand::ListAgents => Err(OrchestrationError::InvalidMessage(
                "agent-list requests do not enter the orchestration queue".to_owned(),
            )),
        }
    }

    pub fn cancel_task(
        &mut self,
        workspaces: &mut WorkspaceManager,
        workspace_id: WorkspaceId,
        task_id: TaskId,
        cancelled_at: Timestamp,
    ) -> Result<(), OrchestrationError> {
        let before = task_handoff(workspace(workspaces, workspace_id)?, task_id)?.clone();
        let root_message_id = before
            .message_id()
            .expect("orchestrated task handoffs have message IDs")
            .clone();
        let cancellation_id = recovery_message_id("cancel", workspace_id, task_id, cancelled_at)?;
        let mut after = before.clone();
        after.cancel(
            cancellation_id.clone(),
            content("Cancelled by the user")?,
            cancelled_at,
        )?;
        execute_task_cancellation(
            workspaces,
            workspace_id,
            task_id,
            before.clone(),
            after,
            cancelled_at,
        )?;
        self.remove_pending(workspace_id, &root_message_id);
        self.enqueue(
            workspaces,
            workspace_id,
            before.id(),
            &cancellation_id,
            cancelled_at,
        )
    }

    pub fn resume_task(
        &mut self,
        workspaces: &mut WorkspaceManager,
        workspace_id: WorkspaceId,
        task_id: TaskId,
        resumed_at: Timestamp,
    ) -> Result<(), OrchestrationError> {
        let state = task_state(workspaces, workspace_id, task_id)?;
        if state != TaskState::Blocked {
            return Err(invalid_task_state(task_id, state, "resume"));
        }
        let workspace = workspace(workspaces, workspace_id)?;
        let previous = task_handoff(workspace, task_id)?.clone();
        let (handoff_id, message_id) =
            if previous.response().is_none() && previous.termination().is_none() {
                let message_id = previous
                    .message_id()
                    .expect("orchestrated task handoffs have message IDs")
                    .clone();
                transition_task(
                    workspaces,
                    workspace_id,
                    task_id,
                    TaskState::Running,
                    resumed_at,
                )?;
                (previous.id(), message_id)
            } else {
                delivery_mechanism(workspace, previous.recipient())?;
                let handoff_id = next_handoff_id(workspace)?;
                let message_id = recovery_message_id("resume", workspace_id, task_id, resumed_at)?;
                let handoff = Handoff::tracked(
                    handoff_id,
                    message_id.clone(),
                    previous.source(),
                    previous.recipient(),
                    HandoffPayload::Task(task_id),
                    None,
                    resumed_at,
                    None,
                )?;
                workspaces.execute(
                    workspace_id,
                    DomainCommand::ResumeTask { task_id, handoff },
                    resumed_at,
                )?;
                (handoff_id, message_id)
            };
        self.enqueue(
            workspaces,
            workspace_id,
            handoff_id,
            &message_id,
            resumed_at,
        )
    }

    pub fn retry_task(
        &mut self,
        workspaces: &mut WorkspaceManager,
        workspace_id: WorkspaceId,
        task_id: TaskId,
        retried_at: Timestamp,
    ) -> Result<TaskId, OrchestrationError> {
        let workspace = workspace(workspaces, workspace_id)?;
        let original_task = workspace
            .task(task_id)
            .ok_or_else(|| {
                OrchestrationError::InvalidMessage(format!("task {task_id} is missing"))
            })?
            .clone();
        if !matches!(
            original_task.state(),
            TaskState::Failed | TaskState::Cancelled
        ) {
            return Err(invalid_task_state(task_id, original_task.state(), "retry"));
        }
        let original_handoff = task_handoff(workspace, task_id)?.clone();
        delivery_mechanism(workspace, original_handoff.recipient())?;
        let retry_task_id = next_task_id(workspace)?;
        let retry_handoff_id = next_handoff_id(workspace)?;
        let retry_message_id =
            recovery_message_id("retry", workspace_id, retry_task_id, retried_at)?;
        let retry = Task::new(
            retry_task_id,
            original_task.title().clone(),
            original_task.prompt().clone(),
            original_task.assignee(),
            Some(task_id),
        );
        let handoff = Handoff::tracked(
            retry_handoff_id,
            retry_message_id.clone(),
            original_handoff.source(),
            original_handoff.recipient(),
            HandoffPayload::Task(retry_task_id),
            None,
            retried_at,
            None,
        )?;
        workspaces.execute(
            workspace_id,
            DomainCommand::AddTaskHandoff {
                task: retry,
                handoff,
            },
            retried_at,
        )?;
        self.enqueue(
            workspaces,
            workspace_id,
            retry_handoff_id,
            &retry_message_id,
            retried_at,
        )?;
        Ok(retry_task_id)
    }

    #[allow(clippy::too_many_arguments)]
    fn accept_handoff(
        &mut self,
        workspaces: &mut WorkspaceManager,
        workspace_id: WorkspaceId,
        source: AgentId,
        recipient: AgentId,
        message_id: &MessageId,
        kind: HandoffKind,
        title: Option<&String>,
        body: &str,
        parent_message_id: Option<&MessageId>,
        response_timeout_ms: Option<u64>,
        accepted_at: Timestamp,
    ) -> Result<(), OrchestrationError> {
        let handoff_message_id = domain_message_id(message_id)?;
        if let Some(existing) = find_handoff(workspaces, workspace_id, &handoff_message_id) {
            let handoff_id = existing.id();
            return self.enqueue(
                workspaces,
                workspace_id,
                handoff_id,
                &handoff_message_id,
                accepted_at,
            );
        }

        let workspace = workspace(workspaces, workspace_id)?;
        delivery_mechanism(workspace, recipient)?;
        let handoff_id = next_handoff_id(workspace)?;
        let parent = parent_message_id
            .map(domain_message_id)
            .transpose()?
            .map(|parent| {
                find_handoff(workspaces, workspace_id, &parent)
                    .map(Handoff::id)
                    .ok_or_else(|| {
                        OrchestrationError::InvalidMessage(
                            "parent handoff does not exist in the workspace".to_owned(),
                        )
                    })
            })
            .transpose()?;
        let response_deadline = response_timeout_ms
            .map(|timeout| checked_timestamp_add(accepted_at, timeout, "response deadline"))
            .transpose()?;

        let command = match kind {
            HandoffKind::Task => {
                let task_id = next_task_id(workspace)?;
                let task = Task::new(
                    task_id,
                    Name::new(
                        title
                            .ok_or_else(|| {
                                OrchestrationError::InvalidMessage(
                                    "task handoff is missing a title".to_owned(),
                                )
                            })?
                            .to_owned(),
                    )?,
                    Content::new(body.to_owned())?,
                    Some(recipient),
                    None,
                );
                let payload = HandoffPayload::Task(task_id);
                let handoff = Handoff::tracked(
                    handoff_id,
                    handoff_message_id.clone(),
                    source,
                    recipient,
                    payload.clone(),
                    parent,
                    accepted_at,
                    response_deadline,
                )?;
                DomainCommand::AddTaskHandoff { task, handoff }
            }
            HandoffKind::Question => {
                let payload = HandoffPayload::Question(Content::new(body.to_owned())?);
                let handoff = Handoff::tracked(
                    handoff_id,
                    handoff_message_id.clone(),
                    source,
                    recipient,
                    payload.clone(),
                    parent,
                    accepted_at,
                    response_deadline,
                )?;
                DomainCommand::AddHandoff(handoff)
            }
        };
        workspaces.execute(workspace_id, command, accepted_at)?;
        self.enqueue(
            workspaces,
            workspace_id,
            handoff_id,
            &handoff_message_id,
            accepted_at,
        )
    }

    fn accept_progress(
        &mut self,
        workspaces: &mut WorkspaceManager,
        accepted: &AcceptedMessage,
        message_id: &MessageId,
        handoff_message_id: &MessageId,
        body: &str,
        accepted_at: Timestamp,
    ) -> Result<(), OrchestrationError> {
        let workspace_id = WorkspaceId::new(accepted.workspace_id);
        let root_id = domain_message_id(handoff_message_id)?;
        let progress_id = domain_message_id(message_id)?;
        let handoff_id = find_handoff(workspaces, workspace_id, &root_id)
            .map(Handoff::id)
            .ok_or_else(|| OrchestrationError::UnknownHandoff(root_id.to_string()))?;
        let before = handoff(workspaces, workspace_id, handoff_id)?.clone();
        validate_routed_participants(&before, accepted, true)?;
        if !before
            .progress()
            .iter()
            .any(|progress| progress.message_id() == &progress_id)
        {
            let mut after = before.clone();
            after.report_progress(HandoffProgress::new(
                progress_id.clone(),
                Content::new(body.to_owned())?,
                accepted_at,
            ))?;
            execute_handoff_update(workspaces, workspace_id, before, after, accepted_at)?;
        }
        if let HandoffPayload::Task(task_id) =
            handoff(workspaces, workspace_id, handoff_id)?.payload()
        {
            transition_task_running(workspaces, workspace_id, *task_id, accepted_at)?;
        }
        self.enqueue(
            workspaces,
            workspace_id,
            handoff_id,
            &progress_id,
            accepted_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn accept_response(
        &mut self,
        workspaces: &mut WorkspaceManager,
        workspace_id: WorkspaceId,
        accepted: &AcceptedMessage,
        message_id: &MessageId,
        handoff_message_id: &MessageId,
        status: ResponseStatus,
        body: &str,
        accepted_at: Timestamp,
    ) -> Result<(), OrchestrationError> {
        let root_id = domain_message_id(handoff_message_id)?;
        let response_id = domain_message_id(message_id)?;
        let handoff_id = find_handoff(workspaces, workspace_id, &root_id)
            .map(Handoff::id)
            .ok_or_else(|| OrchestrationError::UnknownHandoff(root_id.to_string()))?;
        let before = handoff(workspaces, workspace_id, handoff_id)?.clone();
        validate_routed_participants(&before, accepted, true)?;
        if before
            .response()
            .is_none_or(|response| response.message_id() != &response_id)
        {
            let mut after = before.clone();
            after.respond(HandoffResponse::new(
                response_id.clone(),
                response_status(status),
                Content::new(body.to_owned())?,
                accepted_at,
            ))?;
            execute_handoff_update(workspaces, workspace_id, before, after, accepted_at)?;
        }
        if let HandoffPayload::Task(task_id) =
            handoff(workspaces, workspace_id, handoff_id)?.payload()
        {
            transition_task_response(workspaces, workspace_id, *task_id, status, accepted_at)?;
        }
        self.enqueue(
            workspaces,
            workspace_id,
            handoff_id,
            &response_id,
            accepted_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn accept_cancellation(
        &mut self,
        workspaces: &mut WorkspaceManager,
        workspace_id: WorkspaceId,
        accepted: &AcceptedMessage,
        message_id: &MessageId,
        handoff_message_id: &MessageId,
        reason: &str,
        accepted_at: Timestamp,
    ) -> Result<(), OrchestrationError> {
        let root_id = domain_message_id(handoff_message_id)?;
        let cancellation_id = domain_message_id(message_id)?;
        let handoff_id = find_handoff(workspaces, workspace_id, &root_id)
            .map(Handoff::id)
            .ok_or_else(|| OrchestrationError::UnknownHandoff(root_id.to_string()))?;
        let before = handoff(workspaces, workspace_id, handoff_id)?.clone();
        validate_routed_participants(&before, accepted, false)?;
        let task_id = match before.payload() {
            HandoffPayload::Task(task_id) => Some(*task_id),
            HandoffPayload::Question(_) => None,
        };
        if before.termination().is_none() {
            let mut after = before.clone();
            after.cancel(
                cancellation_id.clone(),
                Content::new(reason.to_owned())?,
                accepted_at,
            )?;
            if let Some(task_id) = task_id {
                execute_task_cancellation(
                    workspaces,
                    workspace_id,
                    task_id,
                    before,
                    after,
                    accepted_at,
                )?;
            } else {
                execute_handoff_update(workspaces, workspace_id, before, after, accepted_at)?;
            }
        } else if let Some(task_id) = task_id {
            transition_task_cancelled(workspaces, workspace_id, task_id, accepted_at)?;
        }
        self.remove_pending(workspace_id, &root_id);
        self.enqueue(
            workspaces,
            workspace_id,
            handoff_id,
            &cancellation_id,
            accepted_at,
        )
    }

    pub fn prepare_next(
        &mut self,
        workspaces: &mut WorkspaceManager,
        now: Timestamp,
    ) -> Result<Option<DeliveryRequest>, OrchestrationError> {
        loop {
            let pending = self
                .mailboxes
                .values()
                .filter_map(|mailbox| mailbox.front())
                .find(|pending| pending.next_attempt_at <= now)
                .cloned();
            let Some(pending) = pending else {
                return Ok(None);
            };
            let before = handoff(workspaces, pending.workspace_id, pending.handoff_id)?.clone();
            if before.message_id() == Some(&pending.message_id)
                && before.termination().is_some()
                && !before.is_delivered()
            {
                self.remove_pending(pending.workspace_id, &pending.message_id);
                continue;
            }
            let mut after = before.clone();
            let attempt_ordinal =
                after.begin_delivery(pending.message_id.clone(), pending.mechanism, now)?;
            execute_handoff_update(workspaces, pending.workspace_id, before, after, now)?;
            let request = DeliveryRequest {
                pending,
                attempt_ordinal,
            };
            if now >= request.pending.deadline {
                self.finish_delivery(
                    workspaces,
                    &request,
                    Err("delivery deadline expired".to_owned()),
                    now,
                )?;
                continue;
            }
            return Ok(Some(request));
        }
    }

    pub fn finish_delivery(
        &mut self,
        workspaces: &mut WorkspaceManager,
        request: &DeliveryRequest,
        result: Result<(), String>,
        finished_at: Timestamp,
    ) -> Result<(), OrchestrationError> {
        let before = handoff(
            workspaces,
            request.pending.workspace_id,
            request.pending.handoff_id,
        )?
        .clone();
        let mut after = before.clone();
        let delivered = result.is_ok();
        let retryable = !delivered && finished_at < request.pending.deadline;
        match result {
            Ok(()) => after.complete_delivery(request.attempt_ordinal, finished_at)?,
            Err(error) => after.fail_delivery(
                request.attempt_ordinal,
                finished_at,
                content(if error.trim().is_empty() {
                    "terminal delivery failed"
                } else {
                    &error
                })?,
                retryable,
            )?,
        }
        execute_handoff_update(
            workspaces,
            request.pending.workspace_id,
            before,
            after,
            finished_at,
        )?;

        if delivered {
            let current = handoff(
                workspaces,
                request.pending.workspace_id,
                request.pending.handoff_id,
            )?;
            if current.message_id() == Some(&request.pending.message_id)
                && let HandoffPayload::Task(task_id) = current.payload()
            {
                transition_task_delivered(
                    workspaces,
                    request.pending.workspace_id,
                    *task_id,
                    finished_at,
                )?;
            }
        } else if !retryable {
            let current = handoff(
                workspaces,
                request.pending.workspace_id,
                request.pending.handoff_id,
            )?;
            if current.message_id() == Some(&request.pending.message_id)
                && let HandoffPayload::Task(task_id) = current.payload()
            {
                transition_task_failed(
                    workspaces,
                    request.pending.workspace_id,
                    *task_id,
                    finished_at,
                )?;
            }
        }

        let key = MailboxKey {
            workspace_id: request.pending.workspace_id,
            recipient_agent_id: request.pending.recipient_agent_id,
        };
        let mailbox = self
            .mailboxes
            .get_mut(&key)
            .ok_or_else(|| OrchestrationError::InvalidMessage("mailbox disappeared".to_owned()))?;
        let front = mailbox
            .front_mut()
            .filter(|pending| pending.message_id == request.pending.message_id)
            .ok_or_else(|| {
                OrchestrationError::InvalidMessage(
                    "mailbox order changed during delivery".to_owned(),
                )
            })?;
        if delivered || !retryable {
            mailbox.pop_front();
        } else {
            front.failures = front.failures.saturating_add(1);
            let shift = front.failures.saturating_sub(1).min(MAX_BACKOFF_SHIFT);
            let delay = INITIAL_RETRY_MS.saturating_mul(1_u64 << shift);
            front.next_attempt_at = checked_timestamp_add(finished_at, delay, "retry time")?;
        }
        if mailbox.is_empty() {
            self.mailboxes.remove(&key);
        }
        Ok(())
    }

    pub fn expire_due(
        &mut self,
        workspaces: &mut WorkspaceManager,
        now: Timestamp,
    ) -> Result<(), OrchestrationError> {
        let due: Vec<_> = workspaces
            .recent_workspaces()
            .flat_map(|workspace| {
                workspace.handoffs().filter_map(move |handoff| {
                    (handoff.response().is_none()
                        && handoff.termination().is_none()
                        && handoff
                            .response_deadline()
                            .is_some_and(|deadline| deadline <= now))
                    .then_some((workspace.id(), handoff.id()))
                })
            })
            .collect();
        for (workspace_id, handoff_id) in due {
            let before = handoff(workspaces, workspace_id, handoff_id)?.clone();
            let message_id = before
                .message_id()
                .expect("handoffs with deadlines have message IDs")
                .clone();
            let mut after = before.clone();
            after.time_out(now)?;
            execute_handoff_update(workspaces, workspace_id, before, after, now)?;
            if let HandoffPayload::Task(task_id) =
                handoff(workspaces, workspace_id, handoff_id)?.payload()
            {
                transition_task_failed(workspaces, workspace_id, *task_id, now)?;
            }
            self.remove_pending(workspace_id, &message_id);
        }
        Ok(())
    }

    fn remove_pending(&mut self, workspace_id: WorkspaceId, message_id: &HandoffMessageId) {
        self.mailboxes.retain(|_, mailbox| {
            mailbox.retain(|pending| {
                pending.workspace_id != workspace_id || &pending.message_id != message_id
            });
            !mailbox.is_empty()
        });
    }

    fn enqueue(
        &mut self,
        workspaces: &WorkspaceManager,
        workspace_id: WorkspaceId,
        handoff_id: HandoffId,
        message_id: &HandoffMessageId,
        now: Timestamp,
    ) -> Result<(), OrchestrationError> {
        if self.mailboxes.values().any(|mailbox| {
            mailbox.iter().any(|pending| {
                pending.workspace_id == workspace_id && &pending.message_id == message_id
            })
        }) {
            return Ok(());
        }
        let workspace = workspace(workspaces, workspace_id)?;
        let handoff = workspace
            .handoff(handoff_id)
            .ok_or_else(|| OrchestrationError::UnknownHandoff(handoff_id.to_string()))?;
        if handoff.is_delivery_terminal(message_id) {
            return Ok(());
        }
        let (recipient_agent_id, accepted_at, prompt) =
            delivery_details(workspace, handoff, message_id)?;
        let mechanism = delivery_mechanism(workspace, recipient_agent_id)?;
        let failures = handoff
            .delivery_attempts()
            .iter()
            .filter(|attempt| {
                attempt.message_id() == message_id
                    && matches!(
                        attempt.outcome(),
                        DeliveryOutcome::Failed {
                            retryable: true,
                            ..
                        }
                    )
            })
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
        let pending = PendingDelivery {
            workspace_id,
            handoff_id,
            message_id: message_id.clone(),
            recipient_agent_id,
            mechanism,
            prompt,
            deadline: checked_timestamp_add(accepted_at, DELIVERY_WINDOW_MS, "delivery deadline")?,
            next_attempt_at: now,
            failures,
        };
        self.mailboxes
            .entry(MailboxKey {
                workspace_id,
                recipient_agent_id,
            })
            .or_default()
            .push_back(pending);
        Ok(())
    }
}

fn delivery_details(
    workspace: &Workspace,
    handoff: &Handoff,
    message_id: &HandoffMessageId,
) -> Result<(AgentId, Timestamp, String), OrchestrationError> {
    if handoff.message_id() == Some(message_id) {
        if handoff.termination().is_some() && !handoff.is_delivered() {
            return Err(OrchestrationError::InvalidMessage(
                "terminal handoff no longer accepts initial delivery".to_owned(),
            ));
        }
        let task = match handoff.payload() {
            HandoffPayload::Task(task_id) => workspace.task(*task_id),
            HandoffPayload::Question(_) => None,
        };
        return Ok((
            handoff.recipient(),
            handoff
                .created_at()
                .expect("orchestrated handoffs have creation times"),
            prompt::handoff(handoff, task),
        ));
    }
    if let Some(progress) = handoff
        .progress()
        .iter()
        .find(|progress| progress.message_id() == message_id)
    {
        return Ok((
            handoff.source(),
            progress.reported_at(),
            prompt::progress(handoff, progress),
        ));
    }
    if let Some(response) = handoff
        .response()
        .filter(|response| response.message_id() == message_id)
    {
        return Ok((
            handoff.source(),
            response.responded_at(),
            prompt::response(handoff, response),
        ));
    }
    if let Some(
        termination @ HandoffTermination::Cancelled {
            message_id: cancellation_id,
            cancelled_at,
            ..
        },
    ) = handoff.termination()
        && cancellation_id == message_id
    {
        return Ok((
            handoff.recipient(),
            *cancelled_at,
            prompt::cancellation(handoff, termination),
        ));
    }
    Err(OrchestrationError::InvalidMessage(format!(
        "message {message_id} is not part of handoff {}",
        handoff.id()
    )))
}

fn delivery_message_ids(handoff: &Handoff) -> Vec<HandoffMessageId> {
    let mut messages = Vec::new();
    if handoff.termination().is_none() || handoff.is_delivered() {
        messages.extend(handoff.message_id().cloned());
    }
    messages.extend(
        handoff
            .progress()
            .iter()
            .map(|progress| progress.message_id().clone()),
    );
    messages.extend(
        handoff
            .response()
            .map(|response| response.message_id().clone()),
    );
    if let Some(HandoffTermination::Cancelled { message_id, .. }) = handoff.termination() {
        messages.push(message_id.clone());
    }
    messages
}

fn delivery_mechanism(
    workspace: &Workspace,
    recipient: AgentId,
) -> Result<DeliveryMechanism, OrchestrationError> {
    let agent = workspace.agent(recipient).ok_or_else(|| {
        OrchestrationError::InvalidMessage(format!("recipient agent {recipient} does not exist"))
    })?;
    match agent.program() {
        AgentProgram::Codex => Ok(DeliveryMechanism::CodexTerminal),
        AgentProgram::Claude => Ok(DeliveryMechanism::ClaudeTerminal),
        AgentProgram::OpenCode => Ok(DeliveryMechanism::OpenCodeTerminal),
        AgentProgram::Shell | AgentProgram::Custom(_) => Err(OrchestrationError::UnsafeAdapter {
            agent_id: recipient,
            program: agent.program().label().to_owned(),
        }),
    }
}

fn validate_routed_participants(
    handoff: &Handoff,
    message: &AcceptedMessage,
    from_recipient: bool,
) -> Result<(), OrchestrationError> {
    let (sender, recipient) = if from_recipient {
        (handoff.recipient(), handoff.source())
    } else {
        (handoff.source(), handoff.recipient())
    };
    if message.sender_agent_id != sender.get() || message.recipient_agent_id != recipient.get() {
        return Err(OrchestrationError::InvalidMessage(
            "routed message participants do not match the handoff".to_owned(),
        ));
    }
    Ok(())
}

fn response_status(status: ResponseStatus) -> HandoffResponseStatus {
    match status {
        ResponseStatus::Completed => HandoffResponseStatus::Completed,
        ResponseStatus::Failed => HandoffResponseStatus::Failed,
        ResponseStatus::Blocked => HandoffResponseStatus::Blocked,
    }
}

fn transition_task_delivered(
    workspaces: &mut WorkspaceManager,
    workspace_id: WorkspaceId,
    task_id: TaskId,
    at: Timestamp,
) -> Result<(), OrchestrationError> {
    if task_state(workspaces, workspace_id, task_id)? == TaskState::Queued {
        transition_task(workspaces, workspace_id, task_id, TaskState::Delivered, at)?;
    }
    Ok(())
}

fn transition_task_running(
    workspaces: &mut WorkspaceManager,
    workspace_id: WorkspaceId,
    task_id: TaskId,
    at: Timestamp,
) -> Result<(), OrchestrationError> {
    transition_task_delivered(workspaces, workspace_id, task_id, at)?;
    match task_state(workspaces, workspace_id, task_id)? {
        TaskState::Delivered | TaskState::Blocked => {
            transition_task(workspaces, workspace_id, task_id, TaskState::Running, at)?;
        }
        TaskState::Running => {}
        state => return Err(invalid_task_state(task_id, state, "report progress")),
    }
    Ok(())
}

fn transition_task_response(
    workspaces: &mut WorkspaceManager,
    workspace_id: WorkspaceId,
    task_id: TaskId,
    status: ResponseStatus,
    at: Timestamp,
) -> Result<(), OrchestrationError> {
    match status {
        ResponseStatus::Completed => {
            transition_task_running(workspaces, workspace_id, task_id, at)?;
            transition_task(workspaces, workspace_id, task_id, TaskState::Completed, at)
        }
        ResponseStatus::Blocked => {
            transition_task_running(workspaces, workspace_id, task_id, at)?;
            transition_task(workspaces, workspace_id, task_id, TaskState::Blocked, at)
        }
        ResponseStatus::Failed => {
            transition_task_delivered(workspaces, workspace_id, task_id, at)?;
            match task_state(workspaces, workspace_id, task_id)? {
                TaskState::Delivered | TaskState::Running | TaskState::Blocked => {
                    transition_task(workspaces, workspace_id, task_id, TaskState::Failed, at)
                }
                TaskState::Failed => Ok(()),
                state => Err(invalid_task_state(task_id, state, "fail")),
            }
        }
    }
}

fn transition_task_cancelled(
    workspaces: &mut WorkspaceManager,
    workspace_id: WorkspaceId,
    task_id: TaskId,
    at: Timestamp,
) -> Result<(), OrchestrationError> {
    match task_state(workspaces, workspace_id, task_id)? {
        TaskState::Queued | TaskState::Delivered | TaskState::Running | TaskState::Blocked => {
            transition_task(workspaces, workspace_id, task_id, TaskState::Cancelled, at)
        }
        TaskState::Cancelled => Ok(()),
        state => Err(invalid_task_state(task_id, state, "cancel")),
    }
}

fn transition_task_failed(
    workspaces: &mut WorkspaceManager,
    workspace_id: WorkspaceId,
    task_id: TaskId,
    at: Timestamp,
) -> Result<(), OrchestrationError> {
    match task_state(workspaces, workspace_id, task_id)? {
        TaskState::Queued | TaskState::Delivered | TaskState::Running | TaskState::Blocked => {
            transition_task(workspaces, workspace_id, task_id, TaskState::Failed, at)
        }
        TaskState::Failed => Ok(()),
        state => Err(invalid_task_state(task_id, state, "time out")),
    }
}

fn transition_task(
    workspaces: &mut WorkspaceManager,
    workspace_id: WorkspaceId,
    task_id: TaskId,
    to: TaskState,
    at: Timestamp,
) -> Result<(), OrchestrationError> {
    workspaces.execute(
        workspace_id,
        DomainCommand::TransitionTask { task_id, to },
        at,
    )?;
    Ok(())
}

fn task_state(
    workspaces: &WorkspaceManager,
    workspace_id: WorkspaceId,
    task_id: TaskId,
) -> Result<TaskState, OrchestrationError> {
    workspace(workspaces, workspace_id)?
        .task(task_id)
        .map(Task::state)
        .ok_or_else(|| OrchestrationError::InvalidMessage(format!("task {task_id} is missing")))
}

fn invalid_task_state(task_id: TaskId, state: TaskState, action: &str) -> OrchestrationError {
    OrchestrationError::InvalidMessage(format!(
        "cannot {action} task {task_id} while it is {state}"
    ))
}

fn execute_handoff_update(
    workspaces: &mut WorkspaceManager,
    workspace_id: WorkspaceId,
    before: Handoff,
    after: Handoff,
    at: Timestamp,
) -> Result<(), OrchestrationError> {
    workspaces.execute(
        workspace_id,
        DomainCommand::UpdateHandoff { before, after },
        at,
    )?;
    Ok(())
}

fn execute_task_cancellation(
    workspaces: &mut WorkspaceManager,
    workspace_id: WorkspaceId,
    task_id: TaskId,
    before: Handoff,
    after: Handoff,
    at: Timestamp,
) -> Result<(), OrchestrationError> {
    workspaces.execute(
        workspace_id,
        DomainCommand::CancelTask {
            task_id,
            before,
            after,
        },
        at,
    )?;
    Ok(())
}

fn workspace(
    workspaces: &WorkspaceManager,
    workspace_id: WorkspaceId,
) -> Result<&Workspace, OrchestrationError> {
    workspaces
        .workspace(workspace_id)
        .ok_or(OrchestrationError::UnknownWorkspace(workspace_id))
}

fn handoff(
    workspaces: &WorkspaceManager,
    workspace_id: WorkspaceId,
    handoff_id: HandoffId,
) -> Result<&Handoff, OrchestrationError> {
    workspace(workspaces, workspace_id)?
        .handoff(handoff_id)
        .ok_or_else(|| OrchestrationError::UnknownHandoff(handoff_id.to_string()))
}

fn task_handoff(workspace: &Workspace, task_id: TaskId) -> Result<&Handoff, OrchestrationError> {
    workspace
        .handoffs()
        .filter(|handoff| handoff.payload() == &HandoffPayload::Task(task_id))
        .max_by_key(|handoff| handoff.id())
        .ok_or_else(|| OrchestrationError::InvalidMessage(format!("task {task_id} has no handoff")))
}

fn find_handoff<'a>(
    workspaces: &'a WorkspaceManager,
    workspace_id: WorkspaceId,
    message_id: &HandoffMessageId,
) -> Option<&'a Handoff> {
    workspaces
        .workspace(workspace_id)?
        .handoffs()
        .find(|handoff| handoff.message_id() == Some(message_id))
}

fn next_task_id(workspace: &Workspace) -> Result<TaskId, OrchestrationError> {
    workspace
        .tasks()
        .map(Task::id)
        .map(TaskId::get)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .map(TaskId::new)
        .ok_or(OrchestrationError::IdentifierExhausted("task"))
}

fn next_handoff_id(workspace: &Workspace) -> Result<HandoffId, OrchestrationError> {
    workspace
        .handoffs()
        .map(Handoff::id)
        .map(HandoffId::get)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .map(HandoffId::new)
        .ok_or(OrchestrationError::IdentifierExhausted("handoff"))
}

fn domain_message_id(message_id: &MessageId) -> Result<HandoffMessageId, OrchestrationError> {
    HandoffMessageId::new(message_id.as_str()).map_err(OrchestrationError::from)
}

fn recovery_message_id(
    action: &str,
    workspace_id: WorkspaceId,
    task_id: TaskId,
    at: Timestamp,
) -> Result<HandoffMessageId, OrchestrationError> {
    HandoffMessageId::new(format!(
        "ui-{action}-{}-{}-{}",
        workspace_id.get(),
        task_id.get(),
        at.as_unix_millis()
    ))
    .map_err(OrchestrationError::from)
}

fn checked_timestamp_add(
    timestamp: Timestamp,
    milliseconds: u64,
    field: &'static str,
) -> Result<Timestamp, OrchestrationError> {
    timestamp
        .as_unix_millis()
        .checked_add(milliseconds)
        .map(Timestamp::from_unix_millis)
        .ok_or(OrchestrationError::TimeOverflow(field))
}

fn content(value: &str) -> Result<Content, OrchestrationError> {
    Content::new(value.to_owned()).map_err(OrchestrationError::from)
}

#[derive(Debug)]
pub enum OrchestrationError {
    Workspace(WorkspaceError),
    Validation(crate::domain::ValidationError),
    Handoff(crate::domain::HandoffMutationError),
    UnknownWorkspace(WorkspaceId),
    UnknownHandoff(String),
    UnsafeAdapter { agent_id: AgentId, program: String },
    IdentifierExhausted(&'static str),
    TimeOverflow(&'static str),
    InvalidMessage(String),
}

impl OrchestrationError {
    pub const fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Workspace(WorkspaceError::Persistence(
                PersistenceError::Database { .. } | PersistenceError::Io { .. }
            ))
        )
    }
}

impl Display for OrchestrationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace(source) => source.fmt(formatter),
            Self::Validation(source) => source.fmt(formatter),
            Self::Handoff(source) => source.fmt(formatter),
            Self::UnknownWorkspace(id) => write!(formatter, "workspace {id} is not loaded"),
            Self::UnknownHandoff(id) => write!(formatter, "handoff message {id} does not exist"),
            Self::UnsafeAdapter { agent_id, program } => write!(
                formatter,
                "agent {agent_id} uses {program}, which has no safe automatic prompt delivery"
            ),
            Self::IdentifierExhausted(entity) => {
                write!(formatter, "cannot allocate another {entity} identifier")
            }
            Self::TimeOverflow(field) => write!(formatter, "{field} exceeds the timestamp range"),
            Self::InvalidMessage(message) => formatter.write_str(message),
        }
    }
}

impl Error for OrchestrationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Workspace(source) => Some(source),
            Self::Validation(source) => Some(source),
            Self::Handoff(source) => Some(source),
            Self::UnknownWorkspace(_)
            | Self::UnknownHandoff(_)
            | Self::UnsafeAdapter { .. }
            | Self::IdentifierExhausted(_)
            | Self::TimeOverflow(_)
            | Self::InvalidMessage(_) => None,
        }
    }
}

impl From<WorkspaceError> for OrchestrationError {
    fn from(error: WorkspaceError) -> Self {
        Self::Workspace(error)
    }
}

impl From<crate::domain::ValidationError> for OrchestrationError {
    fn from(error: crate::domain::ValidationError) -> Self {
        Self::Validation(error)
    }
}

impl From<crate::domain::HandoffMutationError> for OrchestrationError {
    fn from(error: crate::domain::HandoffMutationError) -> Self {
        Self::Handoff(error)
    }
}
