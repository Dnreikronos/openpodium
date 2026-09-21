use super::*;

const AGENT_STATES: [AgentState; 6] = [
    AgentState::Starting,
    AgentState::Running,
    AgentState::Waiting,
    AgentState::Completed,
    AgentState::Failed,
    AgentState::Stopped,
];
const ALLOWED_AGENT_TRANSITIONS: [(AgentState, AgentState); 11] = [
    (AgentState::Starting, AgentState::Running),
    (AgentState::Starting, AgentState::Failed),
    (AgentState::Starting, AgentState::Stopped),
    (AgentState::Running, AgentState::Waiting),
    (AgentState::Running, AgentState::Completed),
    (AgentState::Running, AgentState::Failed),
    (AgentState::Running, AgentState::Stopped),
    (AgentState::Waiting, AgentState::Running),
    (AgentState::Waiting, AgentState::Completed),
    (AgentState::Waiting, AgentState::Failed),
    (AgentState::Waiting, AgentState::Stopped),
];
const TASK_STATES: [TaskState; 7] = [
    TaskState::Queued,
    TaskState::Delivered,
    TaskState::Running,
    TaskState::Blocked,
    TaskState::Completed,
    TaskState::Failed,
    TaskState::Cancelled,
];
const ALLOWED_TASK_TRANSITIONS: [(TaskState, TaskState); 13] = [
    (TaskState::Queued, TaskState::Delivered),
    (TaskState::Queued, TaskState::Failed),
    (TaskState::Queued, TaskState::Cancelled),
    (TaskState::Delivered, TaskState::Running),
    (TaskState::Delivered, TaskState::Failed),
    (TaskState::Delivered, TaskState::Cancelled),
    (TaskState::Running, TaskState::Blocked),
    (TaskState::Running, TaskState::Completed),
    (TaskState::Running, TaskState::Failed),
    (TaskState::Running, TaskState::Cancelled),
    (TaskState::Blocked, TaskState::Running),
    (TaskState::Blocked, TaskState::Failed),
    (TaskState::Blocked, TaskState::Cancelled),
];

#[test]
fn default_workspace_is_empty() {
    let workspace = Workspace::default();

    assert_eq!(workspace.name(), "Welcome");
    assert_eq!(workspace.agent_count(), 0);
}

#[test]
fn value_objects_reject_invalid_values() {
    assert_eq!(
        Name::new("  ").unwrap_err().problem(),
        ValidationProblem::Empty
    );
    assert_eq!(
        Name::new("a".repeat(Name::MAX_CHARS + 1))
            .unwrap_err()
            .problem(),
        ValidationProblem::TooLong {
            max_chars: Name::MAX_CHARS
        }
    );
    assert_eq!(
        Content::new("\n\t").unwrap_err().problem(),
        ValidationProblem::Empty
    );
    assert_eq!(
        WorkspaceIcon::new("  ").unwrap_err().problem(),
        ValidationProblem::Empty
    );
    assert_eq!(
        WorkspaceDirectory::new("").unwrap_err().problem(),
        ValidationProblem::Empty
    );
    assert_eq!(
        CanvasPoint::new(f32::NAN, 0.0).unwrap_err().problem(),
        ValidationProblem::NotFinite
    );
    assert_eq!(
        CanvasSize::new(0.0, 100.0).unwrap_err().problem(),
        ValidationProblem::NotPositive
    );
}

#[test]
fn environment_profiles_validate_transport_fields() {
    assert_eq!(
        SshEnvironment::new("-unsafe", None, None, "/workspace")
            .unwrap_err()
            .problem(),
        EnvironmentValidationProblem::InvalidIdentifier
    );
    assert_eq!(
        SshEnvironment::new("example.com", None, Some(0), "/workspace")
            .unwrap_err()
            .problem(),
        EnvironmentValidationProblem::Zero
    );
    assert_eq!(
        ContainerEnvironment::new("docker", "dev container", "/workspace")
            .unwrap_err()
            .problem(),
        EnvironmentValidationProblem::InvalidIdentifier
    );
    assert_eq!(
        CustomEnvironment::new(
            "devbox",
            vec!["bad\0argument".to_owned()],
            WorkspaceDirectory::new("/workspace").unwrap(),
        )
        .unwrap_err()
        .problem(),
        EnvironmentValidationProblem::ControlCharacter
    );
}

#[test]
fn command_presets_and_role_appearance_validate_user_values() {
    let preset = CommandPreset::new(
        CommandPresetId::new(1),
        name("Custom agent"),
        "agent-cli",
        vec!["--prompt".to_owned(), "$(touch /tmp/not-run)".to_owned()],
    )
    .unwrap();
    assert_eq!(preset.arguments()[1], "$(touch /tmp/not-run)");
    assert_eq!(
        CommandPreset::new(
            CommandPresetId::new(1),
            name("Invalid"),
            "bad\0program",
            Vec::new(),
        )
        .unwrap_err()
        .problem(),
        AgentConfigurationProblem::ControlCharacter
    );
    assert_eq!(
        RoleColor::new("purple").unwrap_err().problem(),
        AgentConfigurationProblem::InvalidColor
    );
    assert_eq!(RoleColor::new("#a0bc9f").unwrap().as_str(), "#A0BC9F");
    assert_eq!(
        RoleIcon::new(" ").unwrap_err().problem(),
        AgentConfigurationProblem::Empty
    );
}

#[test]
fn command_presets_are_added_updated_and_removed_explicitly() {
    let mut workspace = test_workspace();
    let id = CommandPresetId::new(1);
    let original = CommandPreset::new(id, name("Agent"), "agent", vec!["run".to_owned()]).unwrap();
    let changed =
        CommandPreset::new(id, name("Agent"), "agent-v2", vec!["run".to_owned()]).unwrap();

    assert_eq!(
        workspace
            .execute(DomainCommand::AddCommandPreset(original.clone()))
            .unwrap(),
        DomainEvent::CommandPresetAdded(original.clone())
    );
    assert_eq!(
        workspace.execute(DomainCommand::UpdateCommandPreset(original.clone())),
        Err(DomainError::UnchangedCommandPreset)
    );
    workspace
        .execute(DomainCommand::UpdateCommandPreset(changed.clone()))
        .unwrap();
    assert_eq!(workspace.command_preset(id), Some(&changed));
    workspace
        .execute(DomainCommand::RemoveCommandPreset(id))
        .unwrap();
    assert!(workspace.command_preset(id).is_none());
}

#[test]
fn custom_agent_programs_require_and_retain_their_command_preset() {
    let mut workspace = test_workspace();
    let preset_id = CommandPresetId::new(1);
    let agent_id = AgentId::new(1);
    let agent = Agent::with_program(
        agent_id,
        name("Custom builder"),
        None,
        AgentProgram::Custom(preset_id),
    );

    assert_eq!(
        workspace.execute(DomainCommand::AddAgent(agent.clone())),
        Err(DomainError::InvalidReference {
            entity: EntityRef::Agent(agent_id),
            field: "program",
            target: EntityRef::CommandPreset(preset_id),
        })
    );

    workspace
        .execute(DomainCommand::AddCommandPreset(
            CommandPreset::new(preset_id, name("Agent"), "agent-cli", Vec::new()).unwrap(),
        ))
        .unwrap();
    workspace.execute(DomainCommand::AddAgent(agent)).unwrap();
    assert_eq!(
        workspace.execute(DomainCommand::RemoveCommandPreset(preset_id)),
        Err(DomainError::CommandPresetInUse {
            preset_id,
            agent_id,
        })
    );
}

#[test]
fn roles_can_change_and_agent_assignments_preserve_references() {
    let mut workspace = test_workspace();
    let role_id = RoleId::new(1);
    let original = Role::with_appearance(
        role_id,
        name("Reviewer"),
        RoleColor::new("#8B5CF6").unwrap(),
        RoleIcon::new("review").unwrap(),
        content("Review changes"),
    );
    workspace
        .execute(DomainCommand::AddRole(original.clone()))
        .unwrap();
    workspace
        .execute(DomainCommand::AddAgent(Agent::new(
            AgentId::new(1),
            name("Ada"),
            None,
        )))
        .unwrap();

    assert_eq!(
        workspace
            .execute(DomainCommand::AssignAgentRole {
                agent_id: AgentId::new(1),
                role_id: Some(role_id),
            })
            .unwrap(),
        DomainEvent::AgentRoleChanged {
            agent_id: AgentId::new(1),
            from: None,
            to: Some(role_id),
        }
    );
    assert_eq!(
        workspace.agent(AgentId::new(1)).unwrap().role_id(),
        Some(role_id)
    );
    assert_eq!(
        workspace.execute(DomainCommand::RemoveRole(role_id)),
        Err(DomainError::RoleInUse {
            role_id,
            agent_id: AgentId::new(1),
        })
    );

    let changed = Role::with_appearance(
        role_id,
        name("Senior reviewer"),
        RoleColor::new("#2563EB").unwrap(),
        RoleIcon::new("shield").unwrap(),
        content("Review correctness and security"),
    );
    workspace
        .execute(DomainCommand::UpdateRole(changed.clone()))
        .unwrap();
    assert_eq!(workspace.role(role_id), Some(&changed));

    workspace
        .execute(DomainCommand::AssignAgentRole {
            agent_id: AgentId::new(1),
            role_id: None,
        })
        .unwrap();
    workspace
        .execute(DomainCommand::RemoveRole(role_id))
        .unwrap();
    assert!(workspace.role(role_id).is_none());
}

#[test]
fn environment_profiles_are_explicit_and_referenced_by_agents() {
    let mut workspace = test_workspace();
    let profile_id = EnvironmentProfileId::new(1);
    let profile = ssh_profile(profile_id, "Remote", "build.example.com");

    assert_eq!(
        workspace
            .execute(DomainCommand::AddEnvironmentProfile(profile.clone()))
            .unwrap(),
        DomainEvent::EnvironmentProfileAdded(profile.clone())
    );
    let agent = Agent::with_program(
        AgentId::new(1),
        name("Remote Codex"),
        None,
        AgentProgram::Codex,
    )
    .in_environment(profile_id);
    workspace
        .execute(DomainCommand::AddAgent(agent.clone()))
        .unwrap();

    assert_eq!(
        workspace.agent(agent.id()).unwrap().environment_id(),
        Some(profile_id)
    );
    assert_eq!(
        workspace.execute(DomainCommand::RemoveEnvironmentProfile(profile_id)),
        Err(DomainError::EnvironmentProfileInUse {
            profile_id,
            agent_id: agent.id(),
        })
    );
}

#[test]
fn environment_profile_updates_reject_no_ops_and_stale_events() {
    let mut workspace = test_workspace();
    let profile_id = EnvironmentProfileId::new(1);
    let original = ssh_profile(profile_id, "Remote", "build.example.com");
    workspace
        .execute(DomainCommand::AddEnvironmentProfile(original.clone()))
        .unwrap();

    assert_eq!(
        workspace.execute(DomainCommand::UpdateEnvironmentProfile(original.clone())),
        Err(DomainError::UnchangedEnvironmentProfile)
    );

    let changed = ssh_profile(profile_id, "Remote", "new.example.com");
    workspace
        .execute(DomainCommand::UpdateEnvironmentProfile(changed.clone()))
        .unwrap();
    let before = workspace.clone();
    assert_eq!(
        workspace.apply(&DomainEvent::EnvironmentProfileChanged {
            from: original,
            to: changed,
        }),
        Err(DomainError::EnvironmentProfileConflict(profile_id))
    );
    assert_eq!(workspace, before);
}

#[test]
fn agents_reject_missing_environment_profiles() {
    let mut workspace = test_workspace();
    let profile_id = EnvironmentProfileId::new(99);
    let agent = Agent::new(AgentId::new(1), name("Remote shell"), None).in_environment(profile_id);

    assert_eq!(
        workspace.execute(DomainCommand::AddAgent(agent)),
        Err(DomainError::InvalidReference {
            entity: EntityRef::Agent(AgentId::new(1)),
            field: "environment_id",
            target: EntityRef::EnvironmentProfile(profile_id),
        })
    );
}

#[test]
fn workspace_settings_changes_are_explicit_and_atomic() {
    let mut workspace = test_workspace();
    let before = workspace.settings().clone();
    let settings = WorkspaceSettings::new(
        name("Renamed workspace"),
        Some(WorkspaceIcon::new("🚀").unwrap()),
        Some(WorkspaceDirectory::new("/projects/openpodium").unwrap()),
        Some(content("Prefer targeted tests")),
    );

    let event = workspace
        .execute(DomainCommand::UpdateWorkspaceSettings(settings.clone()))
        .unwrap();

    assert_eq!(
        event,
        DomainEvent::WorkspaceSettingsChanged {
            from: before,
            to: settings.clone(),
        }
    );
    assert_eq!(workspace.settings(), &settings);
    assert_eq!(workspace.name(), "Renamed workspace");
    assert_eq!(
        workspace.settings().working_directory().unwrap().as_str(),
        "/projects/openpodium"
    );
}

#[test]
fn workspace_settings_reject_no_ops_and_stale_replay() {
    let mut workspace = test_workspace();
    let unchanged = workspace.settings().clone();

    assert_eq!(
        workspace.execute(DomainCommand::UpdateWorkspaceSettings(unchanged.clone())),
        Err(DomainError::UnchangedWorkspaceSettings)
    );

    let before = workspace.clone();
    let stale = DomainEvent::WorkspaceSettingsChanged {
        from: WorkspaceSettings::new(name("Stale"), None, None, None),
        to: unchanged,
    };
    assert_eq!(
        workspace.apply(&stale),
        Err(DomainError::WorkspaceSettingsConflict)
    );
    assert_eq!(workspace, before);
}

#[test]
fn every_agent_transition_is_explicit_and_atomic() {
    let agent_id = AgentId::new(1);

    for from in AGENT_STATES {
        for to in AGENT_STATES {
            let mut workspace = test_workspace();
            let mut agent = Agent::new(agent_id, name("Builder"), None);
            agent.set_state(from);
            workspace.insert_agent_for_test(agent);
            let before = workspace.clone();
            let allowed = ALLOWED_AGENT_TRANSITIONS.contains(&(from, to));

            let result = workspace.execute(DomainCommand::TransitionAgent { agent_id, to });

            assert_eq!(from.can_transition_to(to), allowed, "{from:?} -> {to:?}");
            if allowed {
                assert_eq!(
                    result,
                    Ok(DomainEvent::AgentStateChanged { agent_id, from, to })
                );
                assert_eq!(workspace.agent(agent_id).unwrap().state(), to);
            } else {
                assert_eq!(
                    result,
                    Err(DomainError::InvalidAgentTransition { agent_id, from, to })
                );
                assert_eq!(workspace, before, "{from:?} -> {to:?} mutated state");
            }
        }
    }
}

#[test]
fn every_task_transition_is_explicit_and_atomic() {
    let task_id = TaskId::new(1);

    for from in TASK_STATES {
        for to in TASK_STATES {
            let mut workspace = test_workspace();
            let mut task = Task::new(
                task_id,
                name("Implement domain"),
                content("Define the state machine"),
                None,
                None,
            );
            task.set_state(from);
            workspace.insert_task_for_test(task);
            let before = workspace.clone();
            let allowed = ALLOWED_TASK_TRANSITIONS.contains(&(from, to));

            let result = workspace.execute(DomainCommand::TransitionTask { task_id, to });

            assert_eq!(from.can_transition_to(to), allowed, "{from:?} -> {to:?}");
            if allowed {
                assert_eq!(
                    result,
                    Ok(DomainEvent::TaskStateChanged { task_id, from, to })
                );
                assert_eq!(workspace.task(task_id).unwrap().state(), to);
            } else {
                assert_eq!(
                    result,
                    Err(DomainError::InvalidTaskTransition { task_id, from, to })
                );
                assert_eq!(workspace, before, "{from:?} -> {to:?} mutated state");
            }
        }
    }
}

#[test]
fn commands_validate_references_before_mutating() {
    let mut workspace = test_workspace();
    let agent = Agent::new(AgentId::new(1), name("Builder"), Some(RoleId::new(99)));
    let before = workspace.clone();

    let result = workspace.execute(DomainCommand::AddAgent(agent));

    assert_eq!(
        result,
        Err(DomainError::InvalidReference {
            entity: EntityRef::Agent(AgentId::new(1)),
            field: "role_id",
            target: EntityRef::Role(RoleId::new(99)),
        })
    );
    assert_eq!(workspace, before);
}

#[test]
fn related_entities_and_timeline_events_remain_typed() {
    let mut workspace = test_workspace();
    let role = Role::new(
        RoleId::new(1),
        name("Builder"),
        content("Implement changes"),
    );
    workspace.execute(DomainCommand::AddRole(role)).unwrap();

    for (id, agent_name) in [(AgentId::new(1), "Ada"), (AgentId::new(2), "Linus")] {
        workspace
            .execute(DomainCommand::AddAgent(Agent::new(
                id,
                name(agent_name),
                Some(RoleId::new(1)),
            )))
            .unwrap();
    }

    let task = Task::new(
        TaskId::new(1),
        name("Domain model"),
        content("Implement typed state transitions"),
        Some(AgentId::new(2)),
        None,
    );
    workspace.execute(DomainCommand::AddTask(task)).unwrap();

    let handoff = Handoff::new(
        HandoffId::new(1),
        AgentId::new(1),
        AgentId::new(2),
        HandoffPayload::Task(TaskId::new(1)),
    );
    workspace
        .execute(DomainCommand::AddHandoff(handoff))
        .unwrap();

    let node = Node::new(
        NodeId::new(1),
        NodeTarget::Handoff(HandoffId::new(1)),
        CanvasPoint::new(10.0, 20.0).unwrap(),
        CanvasSize::new(320.0, 240.0).unwrap(),
    );
    let event = workspace.execute(DomainCommand::AddNode(node)).unwrap();
    let timeline_event = TimelineEvent::new(
        TimelineEventId::new(7),
        workspace.id(),
        Timestamp::from_unix_millis(1_000),
        event,
    );

    assert_eq!(timeline_event.id(), TimelineEventId::new(7));
    assert_eq!(timeline_event.workspace_id(), WorkspaceId::new(1));
    assert_eq!(timeline_event.occurred_at().as_unix_millis(), 1_000);
    assert!(workspace.node(NodeId::new(1)).is_some());
}

#[test]
fn retries_preserve_a_terminal_attempt() {
    let mut workspace = test_workspace();
    let original_id = TaskId::new(1);
    let mut original = Task::new(
        original_id,
        name("First attempt"),
        content("Try the change"),
        None,
        None,
    );
    original.set_state(TaskState::Failed);
    workspace.insert_task_for_test(original);
    let retry = Task::new(
        TaskId::new(2),
        name("Second attempt"),
        content("Try the change again"),
        None,
        Some(original_id),
    );

    workspace.execute(DomainCommand::AddTask(retry)).unwrap();

    assert_eq!(
        workspace.task(original_id).unwrap().state(),
        TaskState::Failed
    );
    assert_eq!(
        workspace.task(TaskId::new(2)).unwrap().retry_of(),
        Some(original_id)
    );
}

#[test]
fn typed_task_handoffs_record_atomic_delivery_progress_and_response() {
    let mut workspace = test_workspace();
    for (id, agent_name) in [(1, "Lead"), (2, "Builder")] {
        workspace
            .execute(DomainCommand::AddAgent(Agent::new(
                AgentId::new(id),
                name(agent_name),
                None,
            )))
            .unwrap();
    }
    let task = Task::new(
        TaskId::new(1),
        name("Implement orchestration"),
        content("Build the durable handoff path"),
        Some(AgentId::new(2)),
        None,
    );
    let handoff = Handoff::tracked(
        HandoffId::new(1),
        HandoffMessageId::new("task-1").unwrap(),
        HandoffOrigin::Agent(AgentId::new(1)),
        AgentId::new(2),
        HandoffPayload::Task(task.id()),
        None,
        Timestamp::from_unix_millis(100),
        Some(Timestamp::from_unix_millis(10_000)),
    )
    .unwrap();
    workspace
        .execute(DomainCommand::AddTaskHandoff { task, handoff })
        .unwrap();

    let before = workspace.handoff(HandoffId::new(1)).unwrap().clone();
    let mut started = before.clone();
    assert_eq!(
        started
            .begin_delivery(
                HandoffMessageId::new("task-1").unwrap(),
                DeliveryMechanism::CodexTerminal,
                Timestamp::from_unix_millis(110),
            )
            .unwrap(),
        1
    );
    workspace
        .execute(DomainCommand::UpdateHandoff {
            before,
            after: started.clone(),
        })
        .unwrap();

    let mut delivered = started.clone();
    delivered
        .complete_delivery(1, Timestamp::from_unix_millis(120))
        .unwrap();
    workspace
        .execute(DomainCommand::UpdateHandoff {
            before: started,
            after: delivered.clone(),
        })
        .unwrap();

    let mut progressed = delivered.clone();
    progressed
        .report_progress(HandoffProgress::new(
            HandoffMessageId::new("progress-1").unwrap(),
            content("Running targeted tests"),
            Timestamp::from_unix_millis(130),
        ))
        .unwrap();
    workspace
        .execute(DomainCommand::UpdateHandoff {
            before: delivered,
            after: progressed.clone(),
        })
        .unwrap();

    let mut responded = progressed.clone();
    responded
        .respond(HandoffResponse::new(
            HandoffMessageId::new("response-1").unwrap(),
            HandoffResponseStatus::Completed,
            content("Implementation complete"),
            Timestamp::from_unix_millis(140),
        ))
        .unwrap();
    workspace
        .execute(DomainCommand::UpdateHandoff {
            before: progressed,
            after: responded,
        })
        .unwrap();

    let stored = workspace.handoff(HandoffId::new(1)).unwrap();
    assert!(stored.is_delivered());
    assert_eq!(stored.delivery_attempts().len(), 1);
    assert_eq!(stored.progress().len(), 1);
    assert_eq!(
        stored.response().unwrap().status(),
        HandoffResponseStatus::Completed
    );
}

#[test]
fn task_cancellation_does_not_mutate_the_handoff_when_the_transition_is_invalid() {
    let mut workspace = test_workspace();
    for (id, agent_name) in [(1, "Lead"), (2, "Builder")] {
        workspace
            .execute(DomainCommand::AddAgent(Agent::new(
                AgentId::new(id),
                name(agent_name),
                None,
            )))
            .unwrap();
    }
    let task_id = TaskId::new(1);
    let task = Task::new(
        task_id,
        name("Already complete"),
        content("Keep recovery atomic"),
        Some(AgentId::new(2)),
        None,
    );
    let handoff = Handoff::tracked(
        HandoffId::new(1),
        HandoffMessageId::new("task-1").unwrap(),
        HandoffOrigin::Agent(AgentId::new(1)),
        AgentId::new(2),
        HandoffPayload::Task(task_id),
        None,
        Timestamp::from_unix_millis(100),
        None,
    )
    .unwrap();
    workspace
        .execute(DomainCommand::AddTaskHandoff { task, handoff })
        .unwrap();
    for to in [
        TaskState::Delivered,
        TaskState::Running,
        TaskState::Completed,
    ] {
        workspace
            .execute(DomainCommand::TransitionTask { task_id, to })
            .unwrap();
    }
    let before = workspace.handoff(HandoffId::new(1)).unwrap().clone();
    let mut after = before.clone();
    after
        .cancel(
            HandoffMessageId::new("cancel-1").unwrap(),
            content("Too late"),
            Timestamp::from_unix_millis(200),
        )
        .unwrap();
    let snapshot = workspace.clone();

    let result = workspace.execute(DomainCommand::CancelTask {
        task_id,
        before,
        after,
    });

    assert_eq!(
        result,
        Err(DomainError::InvalidTaskTransition {
            task_id,
            from: TaskState::Completed,
            to: TaskState::Cancelled,
        })
    );
    assert_eq!(workspace, snapshot);
}

#[test]
fn handoff_updates_reject_combined_transitions_and_bound_parent_chains() {
    let mut workspace = test_workspace();
    for (id, agent_name) in [(1, "Lead"), (2, "Builder")] {
        workspace
            .execute(DomainCommand::AddAgent(Agent::new(
                AgentId::new(id),
                name(agent_name),
                None,
            )))
            .unwrap();
    }

    let mut parent = None;
    for raw_id in 1..=MAX_HANDOFF_DEPTH {
        let id = HandoffId::new(u64::try_from(raw_id).unwrap());
        let source = if raw_id % 2 == 0 { 2 } else { 1 };
        let recipient = if source == 1 { 2 } else { 1 };
        let handoff = Handoff::tracked(
            id,
            HandoffMessageId::new(format!("question-{raw_id}")).unwrap(),
            HandoffOrigin::Agent(AgentId::new(source)),
            AgentId::new(recipient),
            HandoffPayload::Question(content("Ask back")),
            parent,
            Timestamp::from_unix_millis(u64::try_from(raw_id).unwrap()),
            None,
        )
        .unwrap();
        workspace
            .execute(DomainCommand::AddHandoff(handoff))
            .unwrap();
        parent = Some(id);
    }

    let too_deep = Handoff::tracked(
        HandoffId::new(17),
        HandoffMessageId::new("question-17").unwrap(),
        HandoffOrigin::Agent(AgentId::new(1)),
        AgentId::new(2),
        HandoffPayload::Question(content("One too many")),
        parent,
        Timestamp::from_unix_millis(17),
        None,
    )
    .unwrap();
    assert_eq!(
        workspace.execute(DomainCommand::AddHandoff(too_deep)),
        Err(DomainError::HandoffChainTooDeep {
            handoff_id: HandoffId::new(17),
            max_depth: MAX_HANDOFF_DEPTH,
        })
    );

    let before = workspace.handoff(HandoffId::new(1)).unwrap().clone();
    let mut after = before.clone();
    after
        .begin_delivery(
            HandoffMessageId::new("question-1").unwrap(),
            DeliveryMechanism::ClaudeTerminal,
            Timestamp::from_unix_millis(20),
        )
        .unwrap();
    after
        .complete_delivery(1, Timestamp::from_unix_millis(21))
        .unwrap();
    assert_eq!(
        workspace.execute(DomainCommand::UpdateHandoff { before, after }),
        Err(DomainError::InvalidHandoff(
            HandoffMutationError::NonAtomicChange
        ))
    );
}

#[test]
fn replay_rejects_stale_state_without_mutating() {
    let mut workspace = test_workspace();
    let agent_id = AgentId::new(1);
    workspace
        .execute(DomainCommand::AddAgent(Agent::new(
            agent_id,
            name("Builder"),
            None,
        )))
        .unwrap();
    let before = workspace.clone();
    let stale_event = TimelineEvent::new(
        TimelineEventId::new(1),
        workspace.id(),
        Timestamp::from_unix_millis(1),
        DomainEvent::AgentStateChanged {
            agent_id,
            from: AgentState::Running,
            to: AgentState::Waiting,
        },
    );

    let result = workspace.replay(&stale_event);

    assert_eq!(
        result,
        Err(DomainError::AgentStateConflict {
            agent_id,
            expected: AgentState::Running,
            actual: AgentState::Starting,
        })
    );
    assert_eq!(workspace, before);
}

#[test]
fn canvas_edits_restore_geometry_groups_and_connections() {
    let mut workspace = test_workspace();
    for (id, name) in [(1, "Codex"), (2, "Claude")] {
        workspace
            .execute(DomainCommand::AddAgent(Agent::with_program(
                AgentId::new(id),
                self::name(name),
                None,
                if id == 1 {
                    AgentProgram::Codex
                } else {
                    AgentProgram::Claude
                },
            )))
            .unwrap();
    }
    let original = CanvasLayout::new(
        vec![
            Node::with_z_index(
                NodeId::new(1),
                NodeTarget::Agent(AgentId::new(1)),
                CanvasPoint::new(0.0, 0.0).unwrap(),
                CanvasSize::new(320.0, 240.0).unwrap(),
                1,
            ),
            Node::with_z_index(
                NodeId::new(2),
                NodeTarget::Agent(AgentId::new(2)),
                CanvasPoint::new(400.0, 0.0).unwrap(),
                CanvasSize::new(320.0, 240.0).unwrap(),
                2,
            ),
        ],
        vec![NodeGroup::new(
            NodeGroupId::new(1),
            [NodeId::new(1), NodeId::new(2)],
        )],
        vec![Connection::new(
            ConnectionId::new(1),
            NodeId::new(1),
            NodeId::new(2),
            ConnectionKind::Coordination,
        )],
    );

    workspace
        .execute(DomainCommand::ReplaceCanvas {
            before: CanvasLayout::default(),
            after: original.clone(),
        })
        .unwrap();
    let moved = CanvasLayout::new(
        original
            .nodes()
            .iter()
            .map(|node| {
                Node::with_content_and_z_index(
                    node.id(),
                    node.content().clone(),
                    CanvasPoint::new(node.position().x() + 40.0, 60.0).unwrap(),
                    node.size(),
                    node.z_index(),
                )
            })
            .collect(),
        original.groups().to_vec(),
        original.connections().to_vec(),
    );

    workspace
        .execute(DomainCommand::ReplaceCanvas {
            before: original.clone(),
            after: moved.clone(),
        })
        .unwrap();
    workspace
        .execute(DomainCommand::ReplaceCanvas {
            before: moved,
            after: original.clone(),
        })
        .unwrap();

    assert_eq!(workspace.canvas_layout(), original);
    assert_eq!(
        workspace.agent(AgentId::new(1)).unwrap().program(),
        AgentProgram::Codex
    );
}

#[test]
fn canvas_edits_reject_dangling_relationships_without_mutating() {
    let mut workspace = test_workspace();
    workspace
        .execute(DomainCommand::AddAgent(Agent::new(
            AgentId::new(1),
            name("Shell"),
            None,
        )))
        .unwrap();
    let node = Node::new(
        NodeId::new(1),
        NodeTarget::Agent(AgentId::new(1)),
        CanvasPoint::new(0.0, 0.0).unwrap(),
        CanvasSize::new(320.0, 240.0).unwrap(),
    );
    workspace
        .execute(DomainCommand::AddNode(node.clone()))
        .unwrap();
    let before = workspace.clone();
    let invalid = CanvasLayout::new(
        vec![node],
        vec![NodeGroup::new(
            NodeGroupId::new(1),
            [NodeId::new(1), NodeId::new(99)],
        )],
        vec![],
    );

    let result = workspace.execute(DomainCommand::ReplaceCanvas {
        before: workspace.canvas_layout(),
        after: invalid,
    });

    assert!(matches!(
        result,
        Err(DomainError::InvalidGroup {
            group_id,
            detail: "a member node does not exist",
        }) if group_id == NodeGroupId::new(1)
    ));
    assert_eq!(workspace, before);
}

#[test]
fn adding_an_agent_and_its_node_is_atomic() {
    let mut workspace = test_workspace();
    let before = workspace.clone();
    let agent = Agent::with_program(AgentId::new(1), name("Codex"), None, AgentProgram::Codex);
    let mismatched_node = Node::new(
        NodeId::new(1),
        NodeTarget::Agent(AgentId::new(2)),
        CanvasPoint::new(0.0, 0.0).unwrap(),
        CanvasSize::new(320.0, 240.0).unwrap(),
    );

    assert!(
        workspace
            .execute(DomainCommand::AddAgentNode {
                agent,
                node: mismatched_node,
            })
            .is_err()
    );
    assert_eq!(workspace, before);
}

// Routine version validation. A stored version is executed unchanged for the
// life of every run that pinned it, so a bad graph must be refused up front.

fn routine_step(
    id: u64,
    depends_on: Vec<u64>,
    bindings: Vec<(&str, RoutineBindingSource)>,
    outputs: Vec<&str>,
) -> RoutineStep {
    RoutineStep::new(
        RoutineStepId::new(id),
        name(&format!("Step {id}")),
        AgentId::new(1),
        content("Work"),
        depends_on.into_iter().map(RoutineStepId::new),
        bindings
            .into_iter()
            .map(|(key, source)| (RoutineInputKey::new(key).unwrap(), source)),
        outputs
            .into_iter()
            .map(|key| RoutineOutputKey::new(key).unwrap()),
        RoutineApproval::NotRequired,
        RoutineRetryPolicy::default(),
        RoutineStepClaims::default(),
    )
    .unwrap()
}

fn routine_version(steps: Vec<RoutineStep>) -> Result<RoutineVersion, RoutineError> {
    RoutineVersion::new(
        RoutineVersionId::new(1),
        RoutineId::new(1),
        1,
        Vec::new(),
        steps,
        None,
        Timestamp::from_unix_millis(1),
    )
}

#[test]
fn routine_versions_reject_dependency_cycles() {
    let cycle = routine_version(vec![
        routine_step(1, vec![2], Vec::new(), Vec::new()),
        routine_step(2, vec![1], Vec::new(), Vec::new()),
    ]);
    assert!(matches!(cycle, Err(RoutineError::DependencyCycle { .. })));

    let self_cycle = RoutineStep::new(
        RoutineStepId::new(1),
        name("Step"),
        AgentId::new(1),
        content("Work"),
        [RoutineStepId::new(1)],
        [],
        [],
        RoutineApproval::NotRequired,
        RoutineRetryPolicy::default(),
        RoutineStepClaims::default(),
    );
    assert!(matches!(
        self_cycle,
        Err(RoutineError::DependencyCycle { .. })
    ));
}

#[test]
fn routine_versions_reject_invalid_output_references() {
    let undeclared = routine_version(vec![
        routine_step(1, Vec::new(), Vec::new(), Vec::new()),
        routine_step(
            2,
            vec![1],
            vec![(
                "report",
                RoutineBindingSource::StepOutput {
                    step_id: RoutineStepId::new(1),
                    key: RoutineOutputKey::new("finding").unwrap(),
                },
            )],
            Vec::new(),
        ),
    ]);
    assert!(matches!(
        undeclared,
        Err(RoutineError::UndeclaredOutputReference { .. })
    ));

    // Reading an output without depending on the step that produces it has no
    // guaranteed ordering, so it is rejected even though the output exists.
    let unordered = routine_version(vec![
        routine_step(1, Vec::new(), Vec::new(), vec!["finding"]),
        routine_step(
            2,
            Vec::new(),
            vec![(
                "report",
                RoutineBindingSource::StepOutput {
                    step_id: RoutineStepId::new(1),
                    key: RoutineOutputKey::new("finding").unwrap(),
                },
            )],
            Vec::new(),
        ),
    ]);
    assert!(matches!(
        unordered,
        Err(RoutineError::UnorderedOutputReference { .. })
    ));

    let missing_dependency =
        routine_version(vec![routine_step(1, vec![9], Vec::new(), Vec::new())]);
    assert!(matches!(
        missing_dependency,
        Err(RoutineError::UnknownDependency { .. })
    ));
}

#[test]
fn routine_versions_accept_a_diamond_and_order_it() {
    let version = routine_version(vec![
        routine_step(1, Vec::new(), Vec::new(), vec!["seed"]),
        routine_step(
            2,
            vec![1],
            vec![(
                "seed",
                RoutineBindingSource::StepOutput {
                    step_id: RoutineStepId::new(1),
                    key: RoutineOutputKey::new("seed").unwrap(),
                },
            )],
            Vec::new(),
        ),
        routine_step(3, vec![1], Vec::new(), Vec::new()),
        routine_step(
            4,
            vec![2, 3],
            vec![(
                "seed",
                RoutineBindingSource::StepOutput {
                    step_id: RoutineStepId::new(1),
                    key: RoutineOutputKey::new("seed").unwrap(),
                },
            )],
            Vec::new(),
        ),
    ])
    .unwrap();
    assert_eq!(version.steps().len(), 4);
}

#[test]
fn routine_prompts_must_bind_every_placeholder() {
    let unbound = routine_version(vec![
        RoutineStep::new(
            RoutineStepId::new(1),
            name("Step"),
            AgentId::new(1),
            content("Build {{target}}"),
            [],
            [],
            [],
            RoutineApproval::NotRequired,
            RoutineRetryPolicy::default(),
            RoutineStepClaims::default(),
        )
        .unwrap(),
    ]);
    assert!(matches!(
        unbound,
        Err(RoutineError::UnboundPlaceholder { .. })
    ));
}

#[test]
fn editing_a_routine_appends_a_version_instead_of_replacing_one() {
    let first = routine_version(vec![routine_step(1, Vec::new(), Vec::new(), Vec::new())]).unwrap();
    let mut routine =
        Routine::new(RoutineId::new(1), name("Nightly"), None, first.clone()).unwrap();
    let second = RoutineVersion::new(
        RoutineVersionId::new(2),
        RoutineId::new(1),
        2,
        Vec::new(),
        vec![routine_step(1, Vec::new(), Vec::new(), Vec::new())],
        None,
        Timestamp::from_unix_millis(2),
    )
    .unwrap();
    routine.push_version(second).unwrap();

    assert_eq!(routine.versions().len(), 2);
    assert_eq!(routine.latest_version().number(), 2);
    assert_eq!(
        routine.version(RoutineVersionId::new(1)),
        Some(&first),
        "an earlier version stays byte-for-byte available to the runs that pinned it"
    );

    // Numbering must follow the stored history.
    let out_of_order = RoutineVersion::new(
        RoutineVersionId::new(3),
        RoutineId::new(1),
        5,
        Vec::new(),
        vec![routine_step(1, Vec::new(), Vec::new(), Vec::new())],
        None,
        Timestamp::from_unix_millis(3),
    )
    .unwrap();
    assert!(matches!(
        routine.push_version(out_of_order),
        Err(RoutineError::InvalidVersionNumber { .. })
    ));
}

#[test]
fn schedule_occurrences_follow_the_stored_timezone_offset() {
    let cadence = RoutineCadence::Daily {
        hour: 9,
        minute: 30,
    };
    let day = 86_400_000_u64;
    // 00:00 UTC on 1970-01-02, in a zone two hours ahead of UTC.
    let from = Timestamp::from_unix_millis(day);
    let next = cadence.next_occurrence(from, 120).unwrap();
    assert_eq!(
        next.as_unix_millis(),
        day + 7 * 3_600_000 + 30 * 60_000,
        "09:30 local in UTC+2 is 07:30 UTC"
    );
    assert!(cadence.next_occurrence(next, 120).unwrap() == next);
    let following = cadence
        .next_occurrence(Timestamp::from_unix_millis(next.as_unix_millis() + 1), 120)
        .unwrap();
    assert_eq!(following.as_unix_millis(), next.as_unix_millis() + day);
}

fn test_workspace() -> Workspace {
    Workspace::new(WorkspaceId::new(1), name("Test workspace"))
}

fn name(value: &str) -> Name {
    Name::new(value).unwrap()
}

fn content(value: &str) -> Content {
    Content::new(value).unwrap()
}

fn ssh_profile(id: EnvironmentProfileId, profile_name: &str, host: &str) -> EnvironmentProfile {
    EnvironmentProfile::new(
        id,
        name(profile_name),
        EnvironmentKind::Ssh(
            SshEnvironment::new(host, Some("builder".to_owned()), Some(22), "/workspace").unwrap(),
        ),
    )
}
