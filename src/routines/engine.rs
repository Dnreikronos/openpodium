use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::Path;

use crate::domain::{
    AgentId, Content, DomainCommand, Handoff, HandoffId, HandoffMessageId, HandoffOrigin,
    HandoffPayload, HandoffResponseStatus, HandoffTermination, Routine, RoutineApprovalDecision,
    RoutineApprovalRecord, RoutineAttempt, RoutineAttemptId, RoutineCheckout, RoutineCheckoutClaim,
    RoutineError, RoutineId, RoutineInputKey, RoutineInterruptionReason, RoutineOccurrenceKey,
    RoutineReservation, RoutineRun, RoutineRunId, RoutineRunPin, RoutineStep, RoutineStepId,
    RoutineStepState, RoutineTransition, RoutineTriggerFiring, RoutineTriggerId, RoutineValue,
    RoutineVersion, Task, TaskId, TaskState, Timestamp, Workspace, WorkspaceDirectory, WorkspaceId,
};
use crate::orchestration::{OrchestrationError, Orchestrator};
use crate::workspaces::{WorkspaceError, WorkspaceManager};

/// What the caller must do after a scheduler tick: register the handoff with
/// the IPC service so the assigned agent can report structured results back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineDispatch {
    workspace_id: WorkspaceId,
    run_id: RoutineRunId,
    step_id: RoutineStepId,
    handoff_id: HandoffId,
    message_id: HandoffMessageId,
    agent_id: AgentId,
}

impl RoutineDispatch {
    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub const fn run_id(&self) -> RoutineRunId {
        self.run_id
    }

    pub const fn step_id(&self) -> RoutineStepId {
        self.step_id
    }

    pub const fn handoff_id(&self) -> HandoffId {
        self.handoff_id
    }

    pub const fn message_id(&self) -> &HandoffMessageId {
        &self.message_id
    }

    pub const fn agent_id(&self) -> AgentId {
        self.agent_id
    }
}

/// Inputs a run start needs that the domain cannot compute for itself.
#[derive(Debug, Clone, Default)]
pub struct RunRequest {
    /// Values the user supplied. An undeclared key here is a mistake and the
    /// run is refused.
    pub inputs: BTreeMap<RoutineInputKey, RoutineValue>,
    /// Context a trigger observed, such as the revisions a Git trigger saw.
    /// Keys the routine does not declare are dropped, because a trigger offers
    /// context to every routine it might start rather than to one of them.
    pub observed: BTreeMap<RoutineInputKey, RoutineValue>,
    pub trigger: Option<TriggerFiring>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerFiring {
    pub trigger_id: RoutineTriggerId,
    pub occurrence: RoutineOccurrenceKey,
    pub next_occurrence: Option<Timestamp>,
}

#[derive(Default)]
pub struct RoutineScheduler {
    /// Handoffs that still need their IPC registration replayed. Registration
    /// is not part of the journal transaction, so a crash between the two is
    /// repaired by re-emitting the dispatch on the next tick.
    pending_registrations: Vec<RoutineDispatch>,
}

impl RoutineScheduler {
    /// How many dispatches still need their IPC registration replayed.
    pub fn pending_registration_count(&self) -> usize {
        self.pending_registrations.len()
    }

    /// Rebuilds scheduler state from durable events.
    ///
    /// A step that was dispatched before the restart has an unknown outcome:
    /// the agent may have finished, may still be working, or may never have
    /// received the prompt. It is marked interrupted and keeps its reservations
    /// until a user resolves it. Steps that were never dispatched simply become
    /// ready again on the next tick.
    pub fn recover(
        workspaces: &mut WorkspaceManager,
        recovered_at: Timestamp,
    ) -> Result<Self, RoutineSchedulerError> {
        let interrupted: Vec<_> = workspaces
            .recent_workspaces()
            .flat_map(|workspace| {
                workspace.active_routine_runs().flat_map(move |run| {
                    run.steps()
                        .filter(|step| step.state() == RoutineStepState::Dispatched)
                        .map(move |step| (workspace.id(), run.id(), step.step_id()))
                })
            })
            .collect();
        for (workspace_id, run_id, step_id) in interrupted {
            advance(
                workspaces,
                workspace_id,
                run_id,
                RoutineTransition::InterruptStep {
                    step_id,
                    reason: RoutineInterruptionReason::DispatchOutcomeUnknown,
                    detected_at: recovered_at,
                },
                recovered_at,
            )?;
        }

        // Re-announce every live routine handoff so the IPC service can route a
        // completion back to it even if its registration was lost.
        let pending_registrations = workspaces
            .recent_workspaces()
            .flat_map(|workspace| {
                workspace.active_routine_runs().flat_map(move |run| {
                    run.steps()
                        .filter(|step| step.state().holds_reservations())
                        .filter_map(move |step| {
                            let attempt = step.attempts().last()?;
                            let handoff = workspace.handoff(attempt.handoff_id())?;
                            Some(RoutineDispatch {
                                workspace_id: workspace.id(),
                                run_id: run.id(),
                                step_id: step.step_id(),
                                handoff_id: attempt.handoff_id(),
                                message_id: handoff.message_id()?.clone(),
                                agent_id: attempt.agent_id(),
                            })
                        })
                })
            })
            .collect();

        Ok(Self {
            pending_registrations,
        })
    }

    /// Starts a run against the routine's latest version.
    ///
    /// Everything the run will need is frozen here: the version, the resolved
    /// inputs, the agent bindings, the canonical checkout for each step, the
    /// Git revision each of those checkouts was on, and the canvas template.
    /// Editing the routine afterwards cannot change what this run does.
    pub fn start_run(
        &mut self,
        workspaces: &mut WorkspaceManager,
        workspace_id: WorkspaceId,
        routine_id: RoutineId,
        request: RunRequest,
        started_at: Timestamp,
    ) -> Result<RoutineRunId, RoutineSchedulerError> {
        let current = workspace(workspaces, workspace_id)?;
        let routine = current
            .routine(routine_id)
            .ok_or(RoutineSchedulerError::UnknownRoutine(routine_id))?;
        let version = routine.latest_version().clone();

        // A trigger contributes its stored inputs; an explicit value wins so a
        // manual start can override what the trigger normally supplies.
        let mut supplied = request.inputs;
        if let Some(firing) = &request.trigger {
            let trigger = routine
                .trigger(firing.trigger_id)
                .ok_or(RoutineSchedulerError::UnknownTrigger(firing.trigger_id))?;
            if !trigger.enabled() {
                return Err(RoutineSchedulerError::TriggerDisabled(firing.trigger_id));
            }
            for (key, value) in trigger.inputs() {
                supplied.entry(key.clone()).or_insert_with(|| value.clone());
            }
            let declared: BTreeSet<_> = version
                .inputs()
                .iter()
                .map(|input| input.key().clone())
                .collect();
            for (key, value) in request.observed {
                if declared.contains(&key) {
                    supplied.entry(key).or_insert(value);
                }
            }
            if current.routine_occurrence_consumed(firing.trigger_id, &firing.occurrence) {
                return Err(RoutineSchedulerError::OccurrenceAlreadyConsumed(
                    firing.occurrence.clone(),
                ));
            }
        }
        // One active run per routine. Repeated triggers coalesce onto the run
        // already in flight instead of racing it for the same resources.
        if let Some(active) = current
            .active_routine_runs()
            .find(|run| run.routine_id() == routine_id)
        {
            return Err(RoutineSchedulerError::RunAlreadyActive {
                routine_id,
                run_id: active.id(),
            });
        }

        let inputs = version.resolve_inputs(&supplied)?;

        // A routine that saved a canvas arrangement rebuilds it here, with
        // fresh entity identifiers. The run then binds its steps to the agents
        // the instantiation created, so the arrangement is genuinely this
        // run's rather than shared with the routine's other runs.
        let bindings = match version.template() {
            Some(template) => {
                let instantiation = workspaces.instantiate_template(
                    workspace_id,
                    template.as_str(),
                    crate::persistence::PointV1 { x: 0.0, y: 0.0 },
                    started_at,
                )?;
                template_bindings(&version, &instantiation.agents)
            }
            None => BTreeMap::new(),
        };
        let refreshed = workspace(workspaces, workspace_id)?;
        let pin = pin_version(refreshed, &version, inputs, &bindings)?;
        let run_id = next_run_id(refreshed)?;
        let run = RoutineRun::start(
            run_id,
            request.trigger.as_ref().map(|firing| firing.trigger_id),
            request
                .trigger
                .as_ref()
                .map(|firing| firing.occurrence.clone()),
            pin,
            started_at,
        );
        let (firing, next_occurrence) = match &request.trigger {
            Some(firing) => (
                Some(RoutineTriggerFiring::new(
                    firing.occurrence.clone(),
                    started_at,
                )),
                firing.next_occurrence,
            ),
            None => (None, None),
        };
        workspaces.execute(
            workspace_id,
            DomainCommand::StartRoutineRun {
                run,
                firing,
                next_occurrence,
            },
            started_at,
        )?;
        Ok(run_id)
    }

    /// Advances every active run: records finished work, then dispatches what
    /// became ready. Returns the dispatches whose IPC registration is pending.
    pub fn tick(
        &mut self,
        workspaces: &mut WorkspaceManager,
        orchestrator: &mut Orchestrator,
        now: Timestamp,
    ) -> Result<Vec<RoutineDispatch>, RoutineSchedulerError> {
        let workspace_ids: Vec<_> = workspaces.recent_workspaces().map(Workspace::id).collect();
        for workspace_id in workspace_ids {
            loop {
                self.observe_finished_steps(workspaces, workspace_id, now)?;
                let dispatched =
                    self.dispatch_ready_steps(workspaces, orchestrator, workspace_id, now)?;
                let settled = self.settle_runs(workspaces, workspace_id, now)?;
                if !dispatched && !settled {
                    break;
                }
            }
        }
        Ok(std::mem::take(&mut self.pending_registrations))
    }

    /// Records a human decision on a gated step.
    #[allow(clippy::too_many_arguments)]
    pub fn decide_approval(
        &mut self,
        workspaces: &mut WorkspaceManager,
        workspace_id: WorkspaceId,
        run_id: RoutineRunId,
        step_id: RoutineStepId,
        decision: RoutineApprovalDecision,
        note: Option<Content>,
        decided_at: Timestamp,
    ) -> Result<(), RoutineSchedulerError> {
        advance(
            workspaces,
            workspace_id,
            run_id,
            RoutineTransition::RecordApproval {
                step_id,
                record: RoutineApprovalRecord::new(decision, decided_at, note),
            },
            decided_at,
        )
    }

    /// Stops further dispatch and asks the orchestrator to cancel active work.
    ///
    /// A cancelled handoff does not prove the agent stopped, so dispatched
    /// steps become interrupted rather than cancelled: they keep holding their
    /// agent and checkout until someone confirms the work is really over.
    pub fn cancel_run(
        &mut self,
        workspaces: &mut WorkspaceManager,
        orchestrator: &mut Orchestrator,
        workspace_id: WorkspaceId,
        run_id: RoutineRunId,
        cancelled_at: Timestamp,
    ) -> Result<(), RoutineSchedulerError> {
        advance(
            workspaces,
            workspace_id,
            run_id,
            RoutineTransition::RequestCancellation { at: cancelled_at },
            cancelled_at,
        )?;

        let active: Vec<_> = run(workspaces, workspace_id, run_id)?
            .steps()
            .filter(|step| step.state() == RoutineStepState::Dispatched)
            .filter_map(|step| {
                step.attempts()
                    .last()
                    .map(|attempt| (step.step_id(), attempt.task_id()))
            })
            .collect();
        for (step_id, task_id) in active {
            // Requesting task cancellation is best effort: the agent may have
            // already finished, and a terminal task is not an error here.
            match orchestrator.cancel_task(workspaces, workspace_id, task_id, cancelled_at) {
                Ok(()) | Err(OrchestrationError::InvalidMessage(_)) => {}
                Err(error) => return Err(error.into()),
            }
            advance(
                workspaces,
                workspace_id,
                run_id,
                RoutineTransition::InterruptStep {
                    step_id,
                    reason: RoutineInterruptionReason::ShutdownUnconfirmed,
                    detected_at: cancelled_at,
                },
                cancelled_at,
            )?;
        }
        self.settle_runs(workspaces, workspace_id, cancelled_at)?;
        Ok(())
    }

    /// Releases an interrupted step after the user confirmed the agent stopped.
    /// The step retries when its budget allows and fails otherwise.
    pub fn resolve_interruption(
        &mut self,
        workspaces: &mut WorkspaceManager,
        workspace_id: WorkspaceId,
        run_id: RoutineRunId,
        step_id: RoutineStepId,
        resolved_at: Timestamp,
    ) -> Result<(), RoutineSchedulerError> {
        advance(
            workspaces,
            workspace_id,
            run_id,
            RoutineTransition::ResolveInterruption {
                step_id,
                finished_at: resolved_at,
            },
            resolved_at,
        )
    }

    /// Reads the durable handoff record of every dispatched step and converts a
    /// finished one into a completion or a failure.
    ///
    /// Only typed IPC results are read. Terminal text is never inspected: an
    /// agent that printed "done" without responding stays dispatched.
    fn observe_finished_steps(
        &mut self,
        workspaces: &mut WorkspaceManager,
        workspace_id: WorkspaceId,
        now: Timestamp,
    ) -> Result<(), RoutineSchedulerError> {
        loop {
            let Some((run_id, transition)) =
                self.next_observation(workspaces, workspace_id, now)?
            else {
                return Ok(());
            };
            advance(workspaces, workspace_id, run_id, transition, now)?;
        }
    }

    fn next_observation(
        &self,
        workspaces: &WorkspaceManager,
        workspace_id: WorkspaceId,
        now: Timestamp,
    ) -> Result<Option<(RoutineRunId, RoutineTransition)>, RoutineSchedulerError> {
        let workspace = workspace(workspaces, workspace_id)?;
        for current in workspace.active_routine_runs() {
            for step in current
                .steps()
                .filter(|step| step.state() == RoutineStepState::Dispatched)
            {
                let Some(attempt) = step.attempts().last() else {
                    continue;
                };
                let Some(handoff) = workspace.handoff(attempt.handoff_id()) else {
                    continue;
                };
                let declared: BTreeSet<_> = current
                    .pin()
                    .version()
                    .step(step.step_id())
                    .map(|definition| definition.outputs().cloned().collect())
                    .unwrap_or_default();

                if let Some(response) = handoff.response() {
                    let transition = match response.status() {
                        HandoffResponseStatus::Completed => {
                            let outputs = response.outputs().clone();
                            let missing: Vec<_> = declared
                                .iter()
                                .filter(|key| !outputs.contains_key(*key))
                                .map(|key| key.as_str().to_owned())
                                .collect();
                            if missing.is_empty() {
                                RoutineTransition::CompleteStep {
                                    step_id: step.step_id(),
                                    outputs,
                                    finished_at: response.responded_at(),
                                }
                            } else {
                                // Completion without the declared outputs is a
                                // failure, not something to infer a value for.
                                RoutineTransition::FailStep {
                                    step_id: step.step_id(),
                                    reason: content(&format!(
                                        "the step completed without its declared output(s): {}",
                                        missing.join(", ")
                                    ))?,
                                    finished_at: response.responded_at(),
                                }
                            }
                        }
                        HandoffResponseStatus::Failed | HandoffResponseStatus::Blocked => {
                            RoutineTransition::FailStep {
                                step_id: step.step_id(),
                                reason: response.body().clone(),
                                finished_at: response.responded_at(),
                            }
                        }
                    };
                    return Ok(Some((current.id(), transition)));
                }

                // A cancelled or timed-out handoff ends the conversation, not
                // necessarily the work. Whoever cancelled it cannot know the
                // agent stopped, so the step is interrupted and keeps holding
                // its claims until a person confirms. Failing it here would
                // release the checkout and start a retry alongside an agent
                // that may still be writing to it.
                if let Some(termination) = handoff.termination() {
                    let at = match termination {
                        HandoffTermination::Cancelled { cancelled_at, .. } => *cancelled_at,
                        HandoffTermination::TimedOut { timed_out_at } => *timed_out_at,
                    };
                    return Ok(Some((
                        current.id(),
                        RoutineTransition::InterruptStep {
                            step_id: step.step_id(),
                            reason: RoutineInterruptionReason::ShutdownUnconfirmed,
                            detected_at: at.max(handoff_start(handoff, now)),
                        },
                    )));
                }
            }
        }
        Ok(None)
    }

    /// Dispatches at most one step per call so each dispatch sees the
    /// reservations the previous one took. Returns whether anything went out.
    fn dispatch_ready_steps(
        &mut self,
        workspaces: &mut WorkspaceManager,
        orchestrator: &mut Orchestrator,
        workspace_id: WorkspaceId,
        now: Timestamp,
    ) -> Result<bool, RoutineSchedulerError> {
        let mut dispatched_any = false;
        loop {
            let Some(plan) = self.next_dispatch(workspaces, workspace_id, now)? else {
                return Ok(dispatched_any);
            };
            match plan {
                Plan::AwaitApproval { run_id, step_id } => {
                    advance(
                        workspaces,
                        workspace_id,
                        run_id,
                        RoutineTransition::AwaitApproval { step_id },
                        now,
                    )?;
                }
                Plan::Dispatch(dispatch) => {
                    // The journal batch records the attempt, its task, its
                    // handoff, and the reservations together. Nothing is sent
                    // anywhere until it commits.
                    workspaces.execute(
                        workspace_id,
                        DomainCommand::DispatchRoutineStep {
                            run_id: dispatch.run_id,
                            step_id: dispatch.step_id,
                            attempt: dispatch.attempt,
                            task: dispatch.task,
                            handoff: dispatch.handoff,
                        },
                        now,
                    )?;
                    orchestrator.submit_routine_handoff(
                        workspaces,
                        workspace_id,
                        dispatch.handoff_id,
                        now,
                    )?;
                    self.pending_registrations.push(RoutineDispatch {
                        workspace_id,
                        run_id: dispatch.run_id,
                        step_id: dispatch.step_id,
                        handoff_id: dispatch.handoff_id,
                        message_id: dispatch.message_id,
                        agent_id: dispatch.agent_id,
                    });
                    dispatched_any = true;
                }
            }
        }
    }

    /// Picks the next action in a stable order: runs oldest first, then steps
    /// by identifier. The same journal always yields the same choice.
    fn next_dispatch(
        &self,
        workspaces: &WorkspaceManager,
        workspace_id: WorkspaceId,
        now: Timestamp,
    ) -> Result<Option<Plan>, RoutineSchedulerError> {
        let workspace = workspace(workspaces, workspace_id)?;
        let held = workspace.held_routine_reservations();
        // One dispatch per call, so a single allocation of each identifier is
        // enough; the caller re-reads the workspace before the next one.
        let next_task = next_task_id(workspace)?;
        let next_handoff = next_handoff_id(workspace)?;
        let next_attempt = next_attempt_id(workspace)?;

        for current in workspace.active_routine_runs() {
            if current.state() != crate::domain::RoutineRunState::Running {
                continue;
            }
            for step_id in current.ready_steps() {
                let step = current
                    .pin()
                    .version()
                    .step(step_id)
                    .ok_or(RoutineError::UnknownStep { step_id })?;
                let run_step = current
                    .step(step_id)
                    .ok_or(RoutineError::UnknownStep { step_id })?;

                if step.approval() == crate::domain::RoutineApproval::Required
                    && run_step.approval().is_none()
                {
                    return Ok(Some(Plan::AwaitApproval {
                        run_id: current.id(),
                        step_id,
                    }));
                }

                let agent_id = current
                    .pin()
                    .agent(step_id)
                    .ok_or(RoutineError::UnpinnedStepBinding { step_id })?;
                let checkout = current
                    .pin()
                    .checkout(step_id)
                    .ok_or(RoutineError::UnpinnedStepCheckout { step_id })?
                    .clone();
                // All declared claims are taken together. A step that cannot
                // take every one waits rather than starting half-isolated.
                let mut required = vec![
                    RoutineReservation::Agent(agent_id),
                    RoutineReservation::Checkout(checkout.clone()),
                ];
                required.extend(
                    step.claims()
                        .resources()
                        .cloned()
                        .map(RoutineReservation::Named),
                );
                if required.iter().any(|claim| held.contains(claim)) {
                    continue;
                }

                let bindings = current.resolve_bindings(step_id)?;
                let prompt = step.render_prompt(&bindings)?;
                // A retry links to its predecessor only when that task ended
                // the way a retry source must. A step can fail while its task
                // legitimately completed — an agent that answered "completed"
                // without the outputs it declared — and claiming a completed
                // task as a retry source would be a lie the domain rejects.
                let previous_task = run_step
                    .attempts()
                    .last()
                    .map(RoutineAttempt::task_id)
                    .filter(|task_id| {
                        workspace.task(*task_id).is_some_and(|task| {
                            matches!(task.state(), TaskState::Failed | TaskState::Cancelled)
                        })
                    });
                let ordinal = u32::try_from(run_step.attempts().len())
                    .ok()
                    .and_then(|count| count.checked_add(1))
                    .ok_or(RoutineError::IdentifierExhausted {
                        entity: "routine attempt",
                    })?;
                let task = Task::new(
                    next_task,
                    step.name().clone(),
                    prompt,
                    Some(agent_id),
                    previous_task,
                );
                let message_id = routine_message_id(current.id(), step_id, ordinal)?;
                let handoff = Handoff::tracked(
                    next_handoff,
                    message_id.clone(),
                    HandoffOrigin::Routine {
                        run_id: current.id(),
                        step_id,
                    },
                    agent_id,
                    HandoffPayload::Task(next_task),
                    None,
                    now,
                    None,
                )
                .map_err(OrchestrationError::from)?;
                let attempt = RoutineAttempt::new(
                    next_attempt,
                    ordinal,
                    next_task,
                    next_handoff,
                    agent_id,
                    checkout,
                    step.claims().resources().cloned(),
                    now,
                )?;
                let dispatch = PlannedDispatch {
                    run_id: current.id(),
                    step_id,
                    handoff_id: next_handoff,
                    message_id,
                    agent_id,
                    attempt,
                    task,
                    handoff,
                };
                return Ok(Some(Plan::Dispatch(Box::new(dispatch))));
            }
        }
        Ok(None)
    }

    /// Finishes runs whose steps all reached a terminal state.
    fn settle_runs(
        &mut self,
        workspaces: &mut WorkspaceManager,
        workspace_id: WorkspaceId,
        now: Timestamp,
    ) -> Result<bool, RoutineSchedulerError> {
        let candidates: Vec<_> = workspace(workspaces, workspace_id)?
            .active_routine_runs()
            .filter(|run| run.steps().all(|step| step.state().is_terminal()))
            .map(RoutineRun::id)
            .collect();
        let settled = !candidates.is_empty();
        for run_id in candidates {
            advance(
                workspaces,
                workspace_id,
                run_id,
                RoutineTransition::Settle { at: now },
                now,
            )?;
        }
        Ok(settled)
    }
}

enum Plan {
    AwaitApproval {
        run_id: RoutineRunId,
        step_id: RoutineStepId,
    },
    Dispatch(Box<PlannedDispatch>),
}

struct PlannedDispatch {
    run_id: RoutineRunId,
    step_id: RoutineStepId,
    handoff_id: HandoffId,
    message_id: HandoffMessageId,
    agent_id: AgentId,
    attempt: RoutineAttempt,
    task: Task,
    handoff: Handoff,
}

/// Maps each step onto the agent its template instantiation created.
///
/// Template documents name agents symbolically as `agent-<id>`, using the
/// identifier the agent had when the template was exported. A step whose agent
/// is not part of the template keeps its own binding.
fn template_bindings(
    version: &RoutineVersion,
    agents: &BTreeMap<String, AgentId>,
) -> BTreeMap<RoutineStepId, AgentId> {
    version
        .steps()
        .iter()
        .filter_map(|step| {
            let symbolic = format!("agent-{}", step.agent_id().get());
            agents.get(&symbolic).map(|agent| (step.id(), *agent))
        })
        .collect()
}

/// Freezes the version, inputs, agent bindings, checkouts, and revisions.
pub fn pin_version(
    workspace: &Workspace,
    version: &RoutineVersion,
    inputs: BTreeMap<RoutineInputKey, RoutineValue>,
    overrides: &BTreeMap<RoutineStepId, AgentId>,
) -> Result<RoutineRunPin, RoutineSchedulerError> {
    let mut agents = BTreeMap::new();
    let mut checkouts = BTreeMap::new();
    let mut revisions = BTreeMap::new();
    for step in version.steps() {
        let agent_id = overrides
            .get(&step.id())
            .copied()
            .unwrap_or(step.agent_id());
        if workspace.agent(agent_id).is_none() {
            return Err(RoutineSchedulerError::UnknownAgent(agent_id));
        }
        agents.insert(step.id(), agent_id);
        let checkout = resolve_checkout(workspace, step, agent_id)?;
        if let Some(revision) = head_revision(&checkout) {
            revisions.insert(checkout.clone(), revision);
        }
        checkouts.insert(step.id(), checkout);
    }
    RoutineRunPin::new(version.clone(), inputs, agents, checkouts, revisions)
        .map_err(RoutineSchedulerError::from)
}

/// Resolves a step's checkout claim to a canonical identity. Two steps conflict
/// exactly when these agree, so the comparison must not depend on how a path
/// was spelled.
fn resolve_checkout(
    workspace: &Workspace,
    step: &RoutineStep,
    agent_id: AgentId,
) -> Result<RoutineCheckout, RoutineSchedulerError> {
    let directory = match step.claims().checkout() {
        RoutineCheckoutClaim::Floor(floor) => workspace
            .floors()
            .entries
            .get(&floor)
            .filter(|entry| entry.lifecycle == crate::domain::FloorLifecycle::Available)
            .map(|entry| entry.directory.clone())
            .ok_or(RoutineSchedulerError::UnavailableCheckout { floor: Some(floor) })?,
        RoutineCheckoutClaim::AgentDefault => {
            let node = workspace
                .all_canvas_layout()
                .nodes()
                .iter()
                .find(|node| node.reference() == Some(crate::domain::NodeTarget::Agent(agent_id)))
                .map(crate::domain::Node::id);
            let directory = match node {
                Some(node) => workspace.node_directory(node).cloned(),
                None => workspace.settings().working_directory().cloned(),
            };
            directory.ok_or(RoutineSchedulerError::UnavailableCheckout { floor: None })?
        }
    };
    Ok(RoutineCheckout::new(canonical_directory(&directory)))
}

/// Normalizes a checkout path so equal checkouts compare equal. Falls back to
/// the stored value when the path cannot be inspected, which keeps a missing
/// checkout comparable with itself rather than silently non-conflicting.
fn canonical_directory(directory: &WorkspaceDirectory) -> WorkspaceDirectory {
    dunce::canonicalize(Path::new(directory.as_str()))
        .ok()
        .and_then(|path| path.to_str().map(str::to_owned))
        .and_then(|path| WorkspaceDirectory::new(path).ok())
        .unwrap_or_else(|| directory.clone())
}

/// Reads the revision a checkout is on, so a run records what it started from.
/// A directory that is not a Git checkout simply records no revision.
fn head_revision(checkout: &RoutineCheckout) -> Option<String> {
    let path = Path::new(checkout.as_str());
    let output = std::process::Command::new("git")
        .current_dir(path)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let revision = String::from_utf8(output.stdout).ok()?;
    let revision = revision.trim();
    (!revision.is_empty()).then(|| revision.to_owned())
}

fn advance(
    workspaces: &mut WorkspaceManager,
    workspace_id: WorkspaceId,
    run_id: RoutineRunId,
    transition: RoutineTransition,
    at: Timestamp,
) -> Result<(), RoutineSchedulerError> {
    workspaces.execute(
        workspace_id,
        DomainCommand::AdvanceRoutineRun { run_id, transition },
        at,
    )?;
    Ok(())
}

fn workspace(
    workspaces: &WorkspaceManager,
    workspace_id: WorkspaceId,
) -> Result<&Workspace, RoutineSchedulerError> {
    workspaces
        .workspace(workspace_id)
        .ok_or(RoutineSchedulerError::UnknownWorkspace(workspace_id))
}

fn run(
    workspaces: &WorkspaceManager,
    workspace_id: WorkspaceId,
    run_id: RoutineRunId,
) -> Result<&RoutineRun, RoutineSchedulerError> {
    workspace(workspaces, workspace_id)?
        .routine_run(run_id)
        .ok_or(RoutineSchedulerError::UnknownRun(run_id))
}

fn handoff_start(handoff: &Handoff, fallback: Timestamp) -> Timestamp {
    handoff.created_at().unwrap_or(fallback)
}

fn next_run_id(workspace: &Workspace) -> Result<RoutineRunId, RoutineSchedulerError> {
    workspace
        .routine_runs()
        .map(RoutineRun::id)
        .map(RoutineRunId::get)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .map(RoutineRunId::new)
        .ok_or(RoutineSchedulerError::Routine(
            RoutineError::IdentifierExhausted {
                entity: "routine run",
            },
        ))
}

fn next_attempt_id(workspace: &Workspace) -> Result<RoutineAttemptId, RoutineSchedulerError> {
    workspace
        .routine_runs()
        .flat_map(RoutineRun::steps)
        .flat_map(|step| step.attempts().iter())
        .map(|attempt| attempt.id().get())
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .map(RoutineAttemptId::new)
        .ok_or(RoutineSchedulerError::Routine(
            RoutineError::IdentifierExhausted {
                entity: "routine attempt",
            },
        ))
}

fn next_task_id(workspace: &Workspace) -> Result<TaskId, RoutineSchedulerError> {
    workspace
        .highest_task_id()
        .checked_add(1)
        .map(TaskId::new)
        .ok_or(RoutineSchedulerError::Routine(
            RoutineError::IdentifierExhausted { entity: "task" },
        ))
}

fn next_handoff_id(workspace: &Workspace) -> Result<HandoffId, RoutineSchedulerError> {
    workspace
        .highest_handoff_id()
        .checked_add(1)
        .map(HandoffId::new)
        .ok_or(RoutineSchedulerError::Routine(
            RoutineError::IdentifierExhausted { entity: "handoff" },
        ))
}

/// Message identifiers are derived from the run, step, and attempt, so a
/// replayed dispatch reuses the same identity instead of creating a second
/// conversation for the same work.
fn routine_message_id(
    run_id: RoutineRunId,
    step_id: RoutineStepId,
    ordinal: u32,
) -> Result<HandoffMessageId, RoutineSchedulerError> {
    HandoffMessageId::new(format!(
        "routine-{}-{}-{ordinal}",
        run_id.get(),
        step_id.get()
    ))
    .map_err(|error| RoutineSchedulerError::Orchestration(OrchestrationError::from(error)))
}

fn content(value: &str) -> Result<Content, RoutineSchedulerError> {
    Content::new(value.to_owned())
        .map_err(|error| RoutineSchedulerError::Orchestration(OrchestrationError::from(error)))
}

#[derive(Debug)]
pub enum RoutineSchedulerError {
    Workspace(WorkspaceError),
    Orchestration(OrchestrationError),
    Routine(RoutineError),
    UnknownWorkspace(WorkspaceId),
    UnknownRoutine(RoutineId),
    UnknownRun(RoutineRunId),
    UnknownTrigger(RoutineTriggerId),
    UnknownAgent(AgentId),
    TriggerDisabled(RoutineTriggerId),
    OccurrenceAlreadyConsumed(RoutineOccurrenceKey),
    RunAlreadyActive {
        routine_id: RoutineId,
        run_id: RoutineRunId,
    },
    UnavailableCheckout {
        floor: Option<u64>,
    },
}

impl RoutineSchedulerError {
    /// Whether retrying the same operation could succeed. Storage faults are
    /// transient; a rejected routine is not.
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::Orchestration(error) => error.is_retryable(),
            Self::Workspace(WorkspaceError::Persistence(
                crate::persistence::PersistenceError::Database { .. }
                | crate::persistence::PersistenceError::Io { .. },
            )) => true,
            _ => false,
        }
    }
}

impl Display for RoutineSchedulerError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace(source) => source.fmt(formatter),
            Self::Orchestration(source) => source.fmt(formatter),
            Self::Routine(source) => source.fmt(formatter),
            Self::UnknownWorkspace(id) => write!(formatter, "workspace {id} is not loaded"),
            Self::UnknownRoutine(id) => write!(formatter, "routine {id} does not exist"),
            Self::UnknownRun(id) => write!(formatter, "routine run {id} does not exist"),
            Self::UnknownTrigger(id) => write!(formatter, "routine trigger {id} does not exist"),
            Self::UnknownAgent(id) => write!(
                formatter,
                "the routine binds agent {id}, which the workspace no longer has"
            ),
            Self::TriggerDisabled(id) => write!(formatter, "routine trigger {id} is disabled"),
            Self::OccurrenceAlreadyConsumed(occurrence) => write!(
                formatter,
                "trigger occurrence {occurrence} already started a run"
            ),
            Self::RunAlreadyActive { routine_id, run_id } => write!(
                formatter,
                "routine {routine_id} is already running as run {run_id}"
            ),
            Self::UnavailableCheckout { floor: Some(floor) } => {
                write!(formatter, "floor {floor} has no available checkout")
            }
            Self::UnavailableCheckout { floor: None } => {
                formatter.write_str("the step's agent has no available checkout")
            }
        }
    }
}

impl Error for RoutineSchedulerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Workspace(source) => Some(source),
            Self::Orchestration(source) => Some(source),
            Self::Routine(source) => Some(source),
            _ => None,
        }
    }
}

impl From<WorkspaceError> for RoutineSchedulerError {
    fn from(error: WorkspaceError) -> Self {
        Self::Workspace(error)
    }
}

impl From<OrchestrationError> for RoutineSchedulerError {
    fn from(error: OrchestrationError) -> Self {
        Self::Orchestration(error)
    }
}

impl From<RoutineError> for RoutineSchedulerError {
    fn from(error: RoutineError) -> Self {
        Self::Routine(error)
    }
}

/// Convenience for callers that build routines: the next free identifiers.
pub fn next_routine_ids(
    workspace: &Workspace,
) -> (RoutineId, crate::domain::RoutineVersionId, RoutineTriggerId) {
    let routine_id = workspace
        .routines()
        .map(Routine::id)
        .map(RoutineId::get)
        .max()
        .unwrap_or(0)
        + 1;
    let version_id = workspace
        .routines()
        .flat_map(|routine| routine.versions().iter())
        .map(|version| version.id().get())
        .max()
        .unwrap_or(0)
        + 1;
    let trigger_id = workspace
        .routines()
        .flat_map(Routine::triggers)
        .map(|trigger| trigger.id().get())
        .max()
        .unwrap_or(0)
        + 1;
    (
        RoutineId::new(routine_id),
        crate::domain::RoutineVersionId::new(version_id),
        RoutineTriggerId::new(trigger_id),
    )
}

/// The step identifier to use for a new step in a routine version.
pub fn next_step_id(version: Option<&RoutineVersion>) -> RoutineStepId {
    RoutineStepId::new(
        version
            .map(|version| {
                version
                    .steps()
                    .iter()
                    .map(|step| step.id().get())
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0)
            + 1,
    )
}
