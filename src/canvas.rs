mod camera;
pub(crate) mod editor;
mod scene;
mod surface;

pub(crate) use camera::Camera;
pub(crate) use editor::{Alignment, History, ZOrder};
pub(crate) use surface::{ConnectionMode, Interaction, Message, view};

use std::collections::{BTreeMap, BTreeSet};

use iced::Color;
use openpodium::accessibility::{CanvasNodeKind, CanvasSemanticSnapshot, RuntimeNodeSemantics};
use openpodium::domain::{
    AgentProgram, CanvasLayout, CanvasNodeContent, NodeId, NodeTarget, Workspace,
};
use openpodium::git::CollisionSeverity;
use openpodium::localization::Localizer;
use openpodium::portal::PortalFrame;

use crate::terminal;

use camera::{ScreenPoint, ViewportSize, WorldPoint, WorldRect};

#[derive(Debug, Clone)]
pub(crate) struct CanvasDocument {
    layout: CanvasLayout,
    labels: BTreeMap<NodeId, NodeLabel>,
    terminals: BTreeMap<NodeId, terminal::View>,
    bodies: BTreeMap<NodeId, String>,
    portal_frames: BTreeMap<NodeId, PortalFrame>,
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
    Context,
}

impl CanvasDocument {
    pub(crate) fn new(
        workspace: &Workspace,
        layout: CanvasLayout,
        terminals: BTreeMap<NodeId, terminal::View>,
        localizer: &Localizer,
    ) -> Self {
        let agent_nodes = layout
            .nodes()
            .iter()
            .filter(|node| {
                matches!(
                    node.content(),
                    CanvasNodeContent::Reference(NodeTarget::Agent(_))
                )
            })
            .map(|node| node.id())
            .collect::<BTreeSet<_>>();
        let semantics =
            CanvasSemanticSnapshot::build(workspace, &layout, &[], localizer, |node_id| {
                terminals.get(&node_id).map_or_else(
                    || RuntimeNodeSemantics {
                        status: agent_nodes
                            .contains(&node_id)
                            .then(|| terminal::Status::Offline.label(localizer)),
                        can_start: agent_nodes.contains(&node_id),
                        ..RuntimeNodeSemantics::default()
                    },
                    |terminal| RuntimeNodeSemantics {
                        title: terminal.title.clone(),
                        status: Some(terminal.status.label(localizer)),
                        value: terminal_text(terminal),
                        can_start: matches!(
                            terminal.status,
                            terminal::Status::Offline
                                | terminal::Status::Exited(_)
                                | terminal::Status::Stopped
                                | terminal::Status::Failed(_)
                        ),
                        can_stop: matches!(
                            terminal.status,
                            terminal::Status::Starting | terminal::Status::Running
                        ),
                    },
                )
            });
        let labels = semantics
            .nodes
            .into_iter()
            .map(|semantic| {
                let kind = match semantic.kind {
                    CanvasNodeKind::Agent => layout
                        .nodes()
                        .iter()
                        .find(|node| node.id() == semantic.id)
                        .and_then(|node| node.content().reference())
                        .and_then(|target| match target {
                            NodeTarget::Agent(agent_id) => workspace.agent(agent_id),
                            NodeTarget::Task(_) | NodeTarget::Handoff(_) => None,
                        })
                        .map_or(
                            NodeKind::Agent {
                                program: AgentProgram::Shell,
                                role_color: None,
                            },
                            |agent| NodeKind::Agent {
                                program: agent.program(),
                                role_color: agent
                                    .role_id()
                                    .and_then(|role_id| workspace.role(role_id))
                                    .map(|role| role_color(role.color().as_str())),
                            },
                        ),
                    CanvasNodeKind::Task => NodeKind::Task,
                    CanvasNodeKind::Handoff => NodeKind::Handoff,
                    CanvasNodeKind::Note
                    | CanvasNodeKind::FileTree
                    | CanvasNodeKind::Artifact
                    | CanvasNodeKind::Diff
                    | CanvasNodeKind::Text
                    | CanvasNodeKind::Portal
                    | CanvasNodeKind::Shape
                    | CanvasNodeKind::Arrow
                    | CanvasNodeKind::Drawing => NodeKind::Context,
                };
                (
                    semantic.id,
                    NodeLabel {
                        title: semantic.name,
                        subtitle: semantic.description,
                        kind,
                    },
                )
            })
            .collect();
        Self {
            layout,
            labels,
            terminals,
            bodies: BTreeMap::new(),
            portal_frames: BTreeMap::new(),
        }
    }

    pub(crate) fn layout(&self) -> &CanvasLayout {
        &self.layout
    }

    pub(crate) fn with_git_severity(
        mut self,
        severities: BTreeMap<NodeId, CollisionSeverity>,
    ) -> Self {
        for (node_id, severity) in severities {
            if let Some(label) = self.labels.get_mut(&node_id) {
                label.subtitle = format!("{} · Git {severity}", label.subtitle);
            }
        }
        self
    }

    pub(crate) fn with_context_bodies(mut self, bodies: BTreeMap<NodeId, String>) -> Self {
        self.bodies = bodies;
        self
    }

    pub(crate) fn with_portal_frames(mut self, frames: BTreeMap<NodeId, PortalFrame>) -> Self {
        self.portal_frames = frames;
        self
    }

    pub(super) fn label(&self, node_id: NodeId) -> &NodeLabel {
        self.labels
            .get(&node_id)
            .expect("every document node receives a label")
    }

    pub(super) fn terminal(&self, node_id: NodeId) -> Option<&terminal::View> {
        self.terminals.get(&node_id)
    }

    pub(super) fn body(&self, node_id: NodeId) -> Option<&str> {
        self.bodies.get(&node_id).map(String::as_str)
    }

    pub(super) fn portal_frame(&self, node_id: NodeId) -> Option<&PortalFrame> {
        self.portal_frames.get(&node_id)
    }
}

fn role_color(value: &str) -> Color {
    let red = u8::from_str_radix(&value[1..3], 16).expect("role colors are validated");
    let green = u8::from_str_radix(&value[3..5], 16).expect("role colors are validated");
    let blue = u8::from_str_radix(&value[5..7], 16).expect("role colors are validated");
    Color::from_rgb8(red, green, blue)
}

fn terminal_text(terminal: &terminal::View) -> Option<String> {
    let mut rows = BTreeMap::<usize, String>::new();
    for cell in &terminal.cells {
        let row = rows.entry(cell.row).or_default();
        if row.len() < cell.column {
            row.push_str(&" ".repeat(cell.column - row.len()));
        }
        row.push_str(&cell.text);
    }
    let text = rows
        .into_values()
        .map(|row| row.trim_end().to_owned())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
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

        let document = CanvasDocument::new(
            &workspace,
            workspace.canvas_layout(),
            BTreeMap::new(),
            &Localizer::new(openpodium::localization::Locale::EnUs),
        )
        .with_git_severity(BTreeMap::from([(
            NodeId::new(1),
            CollisionSeverity::Critical,
        )]));
        let label = document.label(NodeId::new(1));

        assert_eq!(label.title, "review Ada");
        assert_eq!(
            label.subtitle,
            "Reviewer · Codex · terminal offline · Git critical"
        );
        assert!(matches!(
            label.kind,
            NodeKind::Agent {
                role_color: Some(color),
                ..
            } if color == Color::from_rgb8(0x8B, 0x5C, 0xF6)
        ));
    }
}
