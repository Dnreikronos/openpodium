mod camera;
pub(crate) mod editor;
mod scene;
mod surface;

pub(crate) use camera::Camera;
pub(crate) use editor::{Alignment, History, ZOrder};
pub(crate) use surface::{Message, view};

use std::collections::BTreeMap;

use iced::Color;
use openpodium::domain::{AgentProgram, CanvasLayout, NodeId, NodeTarget, Workspace};

use crate::terminal;

use camera::{ScreenPoint, ViewportSize, WorldPoint, WorldRect};

#[derive(Debug, Clone)]
pub(crate) struct CanvasDocument {
    layout: CanvasLayout,
    labels: BTreeMap<NodeId, NodeLabel>,
    terminals: BTreeMap<NodeId, terminal::View>,
}

#[derive(Debug, Clone)]
pub(super) struct NodeLabel {
    pub(super) title: String,
    pub(super) subtitle: String,
    pub(super) kind: NodeKind,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum NodeKind {
    Agent {
        program: AgentProgram,
        role_color: Option<Color>,
    },
    Task,
    Handoff,
}

impl CanvasDocument {
    pub(crate) fn new(
        workspace: &Workspace,
        layout: CanvasLayout,
        terminals: BTreeMap<NodeId, terminal::View>,
    ) -> Self {
        let labels = layout
            .nodes()
            .iter()
            .map(|node| {
                let label = match node.target() {
                    NodeTarget::Agent(agent_id) => workspace.agent(agent_id).map_or_else(
                        || NodeLabel {
                            title: format!("Missing agent {agent_id}"),
                            subtitle: "Unavailable".to_owned(),
                            kind: NodeKind::Agent {
                                program: AgentProgram::Shell,
                                role_color: None,
                            },
                        },
                        |agent| {
                            let status = terminals.get(&node.id()).map_or_else(
                                || "terminal offline".to_owned(),
                                |terminal| terminal.status.label(),
                            );
                            let title = terminals
                                .get(&node.id())
                                .and_then(|terminal| terminal.title.as_deref())
                                .map_or_else(
                                    || agent.name().as_str().to_owned(),
                                    |title| format!("{} — {title}", agent.name().as_str()),
                                );
                            let role = agent.role_id().and_then(|role_id| workspace.role(role_id));
                            NodeLabel {
                                title: role.map_or(title.clone(), |role| {
                                    format!("{} {title}", role.icon())
                                }),
                                subtitle: role.map_or_else(
                                    || format!("{} · {status}", program_name(agent.program())),
                                    |role| {
                                        format!(
                                            "{} · {} · {status}",
                                            role.name(),
                                            program_name(agent.program())
                                        )
                                    },
                                ),
                                kind: NodeKind::Agent {
                                    program: agent.program(),
                                    role_color: role.map(|role| role_color(role.color().as_str())),
                                },
                            }
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
        Self {
            layout,
            labels,
            terminals,
        }
    }

    pub(crate) fn layout(&self) -> &CanvasLayout {
        &self.layout
    }

    pub(super) fn label(&self, node_id: NodeId) -> &NodeLabel {
        self.labels
            .get(&node_id)
            .expect("every document node receives a label")
    }

    pub(super) fn terminal(&self, node_id: NodeId) -> Option<&terminal::View> {
        self.terminals.get(&node_id)
    }
}

fn program_name(program: AgentProgram) -> &'static str {
    program.label()
}

fn role_color(value: &str) -> Color {
    let red = u8::from_str_radix(&value[1..3], 16).expect("role colors are validated");
    let green = u8::from_str_radix(&value[3..5], 16).expect("role colors are validated");
    let blue = u8::from_str_radix(&value[5..7], 16).expect("role colors are validated");
    Color::from_rgb8(red, green, blue)
}

#[cfg(test)]
mod tests {
    use openpodium::domain::{
        Agent, AgentId, CanvasPoint, CanvasSize, Content, DomainCommand, Name, Node, Role,
        RoleColor, RoleIcon, RoleId, WorkspaceId,
    };

    use super::*;

    #[test]
    fn assigned_role_controls_agent_icon_label_and_color() {
        let mut workspace = Workspace::new(WorkspaceId::new(1), Name::new("Test").unwrap());
        let role = Role::with_appearance(
            RoleId::new(1),
            Name::new("Reviewer").unwrap(),
            RoleColor::new("#8B5CF6").unwrap(),
            RoleIcon::new("review").unwrap(),
            Content::new("Review changes").unwrap(),
        );
        workspace.execute(DomainCommand::AddRole(role)).unwrap();
        workspace
            .execute(DomainCommand::AddAgent(Agent::with_program(
                AgentId::new(1),
                Name::new("Ada").unwrap(),
                Some(RoleId::new(1)),
                AgentProgram::Codex,
            )))
            .unwrap();
        let node = Node::new(
            NodeId::new(1),
            NodeTarget::Agent(AgentId::new(1)),
            CanvasPoint::new(0.0, 0.0).unwrap(),
            CanvasSize::new(320.0, 240.0).unwrap(),
        );
        workspace.execute(DomainCommand::AddNode(node)).unwrap();

        let document = CanvasDocument::new(&workspace, workspace.canvas_layout(), BTreeMap::new());
        let label = document.label(NodeId::new(1));

        assert_eq!(label.title, "review Ada");
        assert_eq!(label.subtitle, "Reviewer · Codex · terminal offline");
        assert!(matches!(
            label.kind,
            NodeKind::Agent {
                role_color: Some(color),
                ..
            } if color == Color::from_rgb8(0x8B, 0x5C, 0xF6)
        ));
    }
}
