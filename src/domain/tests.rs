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
const ALLOWED_TASK_TRANSITIONS: [(TaskState, TaskState); 12] = [
    (TaskState::Queued, TaskState::Delivered),
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
                Node::with_z_index(
                    node.id(),
                    node.target(),
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

fn test_workspace() -> Workspace {
    Workspace::new(WorkspaceId::new(1), name("Test workspace"))
}

fn name(value: &str) -> Name {
    Name::new(value).unwrap()
}

fn content(value: &str) -> Content {
    Content::new(value).unwrap()
}
