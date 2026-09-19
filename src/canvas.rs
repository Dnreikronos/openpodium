mod camera;
pub(crate) mod editor;
mod scene;
mod surface;

pub(crate) use camera::Camera;
pub(crate) use editor::{Alignment, History, ZOrder};
pub(crate) use surface::{Message, view};

use std::collections::BTreeMap;

use openpodium::domain::{AgentProgram, CanvasLayout, NodeId, NodeTarget, Workspace};

use camera::{ScreenPoint, ViewportSize, WorldPoint, WorldRect};

#[derive(Debug, Clone)]
pub(crate) struct CanvasDocument {
    layout: CanvasLayout,
    labels: BTreeMap<NodeId, NodeLabel>,
}

#[derive(Debug, Clone)]
pub(super) struct NodeLabel {
    pub(super) title: String,
    pub(super) subtitle: String,
    pub(super) kind: NodeKind,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum NodeKind {
    Agent(AgentProgram),
    Task,
    Handoff,
}

impl CanvasDocument {
    pub(crate) fn new(workspace: &Workspace, layout: CanvasLayout) -> Self {
        let labels = layout
            .nodes()
            .iter()
            .map(|node| {
                let label = match node.target() {
                    NodeTarget::Agent(agent_id) => workspace.agent(agent_id).map_or_else(
                        || NodeLabel {
                            title: format!("Missing agent {agent_id}"),
                            subtitle: "Unavailable".to_owned(),
                            kind: NodeKind::Agent(AgentProgram::Shell),
                        },
                        |agent| NodeLabel {
                            title: agent.name().as_str().to_owned(),
                            subtitle: format!(
                                "{} · terminal offline",
                                program_name(agent.program())
                            ),
                            kind: NodeKind::Agent(agent.program()),
                        },
                    ),
                    NodeTarget::Task(task_id) => workspace.task(task_id).map_or_else(
                        || NodeLabel {
                            title: format!("Missing task {task_id}"),
                            subtitle: "Unavailable".to_owned(),
                            kind: NodeKind::Task,
                        },
                        |task| NodeLabel {
                            title: task.title().as_str().to_owned(),
                            subtitle: task.state().to_string(),
                            kind: NodeKind::Task,
                        },
                    ),
                    NodeTarget::Handoff(handoff_id) => NodeLabel {
                        title: format!("Handoff {handoff_id}"),
                        subtitle: workspace.handoff(handoff_id).map_or_else(
                            || "Unavailable".to_owned(),
                            |handoff| {
                                format!(
                                    "Agent {} → Agent {}",
                                    handoff.source(),
                                    handoff.recipient()
                                )
                            },
                        ),
                        kind: NodeKind::Handoff,
                    },
                };
                (node.id(), label)
            })
            .collect();
        Self { layout, labels }
    }

    pub(crate) fn layout(&self) -> &CanvasLayout {
        &self.layout
    }

    pub(super) fn label(&self, node_id: NodeId) -> &NodeLabel {
        self.labels
            .get(&node_id)
            .expect("every document node receives a label")
    }
}

fn program_name(program: AgentProgram) -> &'static str {
    match program {
        AgentProgram::Codex => "Codex",
        AgentProgram::Claude => "Claude",
        AgentProgram::Shell => "Shell",
    }
}
