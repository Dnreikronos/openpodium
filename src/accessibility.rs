//! Toolkit-independent accessibility projections.
//!
//! These types describe application state without depending on a native
//! accessibility transport. A GUI adapter can translate the snapshot to
//! AccessKit (or another platform API) without owning duplicate state.

use std::collections::BTreeSet;

use crate::domain::{
    CanvasLayout, CanvasNodeContent, Node, NodeId, NodeTarget, ShapeKind, TaskState, Workspace,
};
use crate::localization::Localizer;
use crate::portal::PortalTargetKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasNodeKind {
    Agent,
    Task,
    Handoff,
    Note,
    FileTree,
    Artifact,
    Diff,
    Text,
    Portal,
    Shape,
    Arrow,
    Drawing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasAction {
    Select,
    Activate,
    Edit,
    Start,
    Stop,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeNodeSemantics {
    pub title: Option<String>,
    pub status: Option<String>,
    pub value: Option<String>,
    pub can_start: bool,
    pub can_stop: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasSemanticNode {
    pub id: NodeId,
    pub kind: CanvasNodeKind,
    pub name: String,
    pub description: String,
    pub value: Option<String>,
    pub selected: bool,
    pub position_in_set: usize,
    pub set_size: usize,
    pub actions: Vec<CanvasAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasSemanticSnapshot {
    pub workspace_name: String,
    pub nodes: Vec<CanvasSemanticNode>,
}

impl CanvasSemanticSnapshot {
    pub fn build(
        workspace: &Workspace,
        layout: &CanvasLayout,
        selected: &[NodeId],
        localizer: &Localizer,
        runtime: impl Fn(NodeId) -> RuntimeNodeSemantics,
    ) -> Self {
        let selected = selected.iter().copied().collect::<BTreeSet<_>>();
        let mut nodes = layout.nodes().iter().collect::<Vec<_>>();
        nodes.sort_by_key(|node| {
            (
                ordered_coordinate(node.position().y()),
                ordered_coordinate(node.position().x()),
                node.id(),
            )
        });
        let set_size = nodes.len();
        let nodes = nodes
            .into_iter()
            .enumerate()
            .map(|(index, node)| {
                describe_node(
                    workspace,
                    node,
                    localizer,
                    selected.contains(&node.id()),
                    index + 1,
                    set_size,
                    runtime(node.id()),
                )
            })
            .collect();
        Self {
            workspace_name: workspace.name().to_owned(),
            nodes,
        }
    }

    pub fn node(&self, node_id: NodeId) -> Option<&CanvasSemanticNode> {
        self.nodes.iter().find(|node| node.id == node_id)
    }

    pub fn resolve_action(
        &self,
        node_id: NodeId,
        action: CanvasAction,
    ) -> Option<&CanvasSemanticNode> {
        self.node(node_id)
            .filter(|node| node.actions.contains(&action))
    }
}

fn describe_node(
    workspace: &Workspace,
    node: &Node,
    localizer: &Localizer,
    selected: bool,
    position_in_set: usize,
    set_size: usize,
    runtime: RuntimeNodeSemantics,
) -> CanvasSemanticNode {
    let (kind, name, mut description, mut actions) = match node.content() {
        CanvasNodeContent::Reference(NodeTarget::Agent(agent_id)) => {
            workspace.agent(*agent_id).map_or_else(
                || {
                    (
                        CanvasNodeKind::Agent,
                        localizer.with_str("canvas-missing-agent", "id", agent_id.to_string()),
                        localizer.text("canvas-unavailable"),
                        vec![CanvasAction::Select],
                    )
                },
                |agent| {
                    let role = agent.role_id().and_then(|role_id| workspace.role(role_id));
                    let name = role.map_or_else(
                        || agent.name().as_str().to_owned(),
                        |role| format!("{} {}", role.icon(), agent.name()),
                    );
                    let description = role.map_or_else(
                        || agent.program().label().to_owned(),
                        |role| format!("{} · {}", role.name(), agent.program().label()),
                    );
                    let name = runtime
                        .title
                        .as_deref()
                        .map_or(name.clone(), |title| format!("{name} — {title}"));
                    (
                        CanvasNodeKind::Agent,
                        name,
                        description,
                        vec![CanvasAction::Select, CanvasAction::Activate],
                    )
                },
            )
        }
        CanvasNodeContent::Reference(NodeTarget::Task(task_id)) => {
            workspace.task(*task_id).map_or_else(
                || {
                    (
                        CanvasNodeKind::Task,
                        localizer.with_str("canvas-missing-task", "id", task_id.to_string()),
                        localizer.text("canvas-unavailable"),
                        vec![CanvasAction::Select],
                    )
                },
                |task| {
                    (
                        CanvasNodeKind::Task,
                        task.title().as_str().to_owned(),
                        localizer.text(task_state_message(task.state())),
                        vec![CanvasAction::Select, CanvasAction::Activate],
                    )
                },
            )
        }
        CanvasNodeContent::Reference(NodeTarget::Handoff(handoff_id)) => {
            workspace.handoff(*handoff_id).map_or_else(
                || {
                    (
                        CanvasNodeKind::Handoff,
                        localizer.with_str("canvas-handoff", "id", handoff_id.to_string()),
                        localizer.text("canvas-unavailable"),
                        vec![CanvasAction::Select],
                    )
                },
                |handoff| {
                    (
                        CanvasNodeKind::Handoff,
                        localizer.with_str("canvas-handoff", "id", handoff_id.to_string()),
                        localizer.with_str(
                            "canvas-handoff-recipient",
                            "recipient",
                            handoff.recipient().to_string(),
                        ),
                        vec![CanvasAction::Select, CanvasAction::Activate],
                    )
                },
            )
        }
        CanvasNodeContent::Note { path, title } => (
            CanvasNodeKind::Note,
            title.as_str().to_owned(),
            path.as_str().to_owned(),
            vec![
                CanvasAction::Select,
                CanvasAction::Activate,
                CanvasAction::Edit,
            ],
        ),
        CanvasNodeContent::FileTree { root } => (
            CanvasNodeKind::FileTree,
            localizer.text("canvas-project-files"),
            root.as_str().to_owned(),
            vec![CanvasAction::Select, CanvasAction::Activate],
        ),
        CanvasNodeContent::Artifact { path } => (
            CanvasNodeKind::Artifact,
            path.as_str()
                .rsplit('/')
                .next()
                .unwrap_or(path.as_str())
                .to_owned(),
            path.as_str().to_owned(),
            vec![CanvasAction::Select, CanvasAction::Activate],
        ),
        CanvasNodeContent::Diff { path, .. } => (
            CanvasNodeKind::Diff,
            localizer.with_str("canvas-diff", "path", path.as_str()),
            localizer.text("canvas-working-tree-against-head"),
            vec![CanvasAction::Select, CanvasAction::Activate],
        ),
        CanvasNodeContent::Text { .. } => (
            CanvasNodeKind::Text,
            localizer.text("canvas-text"),
            localizer.text("canvas-annotation"),
            vec![
                CanvasAction::Select,
                CanvasAction::Activate,
                CanvasAction::Edit,
            ],
        ),
        CanvasNodeContent::Portal(config) => (
            CanvasNodeKind::Portal,
            localizer.text("canvas-portal"),
            format!(
                "{} · {}",
                localizer.text(portal_kind_message(config.target().kind())),
                config.target().selector()
            ),
            vec![CanvasAction::Select, CanvasAction::Activate],
        ),
        CanvasNodeContent::Shape(shape) => (
            CanvasNodeKind::Shape,
            localizer.text(shape_kind_message(shape.kind())),
            localizer.text("canvas-shape-description"),
            vec![CanvasAction::Select],
        ),
        CanvasNodeContent::Arrow(_) => (
            CanvasNodeKind::Arrow,
            localizer.text("canvas-arrow"),
            localizer.text("canvas-annotation"),
            vec![CanvasAction::Select],
        ),
        CanvasNodeContent::Freehand(_) => (
            CanvasNodeKind::Drawing,
            localizer.text("canvas-drawing"),
            localizer.text("canvas-freehand-annotation"),
            vec![CanvasAction::Select],
        ),
    };

    if let Some(status) = runtime.status {
        description = format!("{description} · {status}");
    }
    if runtime.can_start {
        actions.push(CanvasAction::Start);
    }
    if runtime.can_stop {
        actions.push(CanvasAction::Stop);
    }

    CanvasSemanticNode {
        id: node.id(),
        kind,
        name,
        description,
        value: runtime.value,
        selected,
        position_in_set,
        set_size,
        actions,
    }
}

const fn task_state_message(state: TaskState) -> &'static str {
    match state {
        TaskState::Queued => "canvas-task-queued",
        TaskState::Delivered => "canvas-task-delivered",
        TaskState::Running => "canvas-task-running",
        TaskState::Blocked => "canvas-task-blocked",
        TaskState::Completed => "canvas-task-completed",
        TaskState::Failed => "canvas-task-failed",
        TaskState::Cancelled => "canvas-task-cancelled",
    }
}

const fn portal_kind_message(kind: PortalTargetKind) -> &'static str {
    match kind {
        PortalTargetKind::Browser => "canvas-portal-browser",
        PortalTargetKind::Android => "canvas-portal-android",
        PortalTargetKind::Ios => "canvas-portal-ios",
    }
}

const fn shape_kind_message(kind: ShapeKind) -> &'static str {
    match kind {
        ShapeKind::Rectangle => "canvas-shape-rectangle",
        ShapeKind::Ellipse => "canvas-shape-ellipse",
    }
}

fn ordered_coordinate(value: f32) -> i64 {
    (f64::from(value) * 1_000.0).round() as i64
}

#[cfg(test)]
mod tests {
    use crate::domain::{
        Agent, AgentId, AgentProgram, CanvasPoint, CanvasSize, DomainCommand, Name, Node,
        WorkspaceId,
    };

    use super::*;
    use crate::localization::Locale;

    #[test]
    fn nodes_follow_reading_order_and_expose_runtime_actions() {
        let mut workspace = Workspace::new(WorkspaceId::new(1), Name::new("Example").unwrap());
        workspace
            .execute(DomainCommand::AddAgent(Agent::with_program(
                AgentId::new(1),
                Name::new("Ada").unwrap(),
                None,
                AgentProgram::Codex,
            )))
            .unwrap();
        for (id, x, y) in [(1, 50.0, 20.0), (2, 10.0, 20.0), (3, 0.0, 5.0)] {
            workspace
                .execute(DomainCommand::AddNode(Node::new(
                    NodeId::new(id),
                    NodeTarget::Agent(AgentId::new(1)),
                    CanvasPoint::new(x, y).unwrap(),
                    CanvasSize::new(320.0, 240.0).unwrap(),
                )))
                .unwrap();
        }

        let snapshot = CanvasSemanticSnapshot::build(
            &workspace,
            &workspace.canvas_layout(),
            &[NodeId::new(2)],
            &Localizer::new(Locale::EnUs),
            |_| RuntimeNodeSemantics {
                status: Some("terminal running".to_owned()),
                can_stop: true,
                ..RuntimeNodeSemantics::default()
            },
        );

        assert_eq!(
            snapshot
                .nodes
                .iter()
                .map(|node| node.id)
                .collect::<Vec<_>>(),
            [NodeId::new(3), NodeId::new(2), NodeId::new(1)]
        );
        assert!(snapshot.node(NodeId::new(2)).unwrap().selected);
        assert_eq!(snapshot.nodes[1].position_in_set, 2);
        assert!(snapshot.nodes[1].actions.contains(&CanvasAction::Stop));
        assert_eq!(snapshot.nodes[1].description, "Codex · terminal running");
        assert!(
            snapshot
                .resolve_action(NodeId::new(2), CanvasAction::Stop)
                .is_some()
        );
        assert!(
            snapshot
                .resolve_action(NodeId::new(2), CanvasAction::Start)
                .is_none()
        );
        assert!(
            snapshot
                .resolve_action(NodeId::new(99), CanvasAction::Select)
                .is_none()
        );
    }

    #[test]
    fn missing_handoffs_are_localized_and_cannot_be_activated() {
        let workspace = Workspace::new(WorkspaceId::new(1), Name::new("Exemplo").unwrap());
        let layout = CanvasLayout::new(
            vec![Node::new(
                NodeId::new(1),
                NodeTarget::Handoff(crate::domain::HandoffId::new(9)),
                CanvasPoint::new(0.0, 0.0).unwrap(),
                CanvasSize::new(320.0, 240.0).unwrap(),
            )],
            Vec::new(),
            Vec::new(),
        );

        let snapshot = CanvasSemanticSnapshot::build(
            &workspace,
            &layout,
            &[],
            &Localizer::new(Locale::PtBr),
            |_| RuntimeNodeSemantics::default(),
        );
        let node = snapshot.node(NodeId::new(1)).unwrap();

        assert_eq!(node.name, "Transferência 9");
        assert_eq!(node.description, "Indisponível");
        assert_eq!(node.actions, [CanvasAction::Select]);
        assert!(
            snapshot
                .resolve_action(NodeId::new(1), CanvasAction::Activate)
                .is_none()
        );
    }
}
