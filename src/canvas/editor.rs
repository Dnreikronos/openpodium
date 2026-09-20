use std::collections::{BTreeMap, BTreeSet};

use openpodium::domain::{
    CanvasLayout, CanvasPoint, CanvasSize, Connection, ConnectionId, ConnectionKind, Node,
    NodeGroup, NodeGroupId, NodeId,
};

const GRID_STEP: f32 = 20.0;
const DUPLICATE_OFFSET: f32 = 40.0;
pub(crate) const MIN_NODE_WIDTH: f32 = 240.0;
pub(crate) const MIN_NODE_HEIGHT: f32 = 160.0;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Alignment {
    HorizontalCenters,
    VerticalCenters,
    DistributeHorizontally,
    DistributeVertically,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ZOrder {
    Front,
    Back,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct History {
    undo: Vec<CanvasLayout>,
    redo: Vec<CanvasLayout>,
}

impl History {
    pub(crate) fn record(&mut self, before: CanvasLayout) {
        self.undo.push(before);
        self.redo.clear();
    }

    pub(crate) fn undo_target(&self) -> Option<&CanvasLayout> {
        self.undo.last()
    }

    pub(crate) fn redo_target(&self) -> Option<&CanvasLayout> {
        self.redo.last()
    }

    pub(crate) fn complete_undo(&mut self, current: CanvasLayout) {
        self.undo.pop();
        self.redo.push(current);
    }

    pub(crate) fn complete_redo(&mut self, current: CanvasLayout) {
        self.redo.pop();
        self.undo.push(current);
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

pub(crate) fn selection_for_click(
    layout: &CanvasLayout,
    current: &[NodeId],
    clicked: NodeId,
    extend: bool,
) -> Vec<NodeId> {
    let clicked = group_members(layout, clicked);
    if !extend {
        return clicked;
    }

    let mut selected = current.to_vec();
    let removes = clicked.iter().all(|node_id| selected.contains(node_id));
    for node_id in clicked {
        if removes {
            selected.retain(|selected_id| *selected_id != node_id);
        } else if !selected.contains(&node_id) {
            selected.push(node_id);
        }
    }
    selected
}

pub(crate) fn move_nodes(
    layout: &CanvasLayout,
    selection: &[NodeId],
    delta_x: f32,
    delta_y: f32,
) -> CanvasLayout {
    let selected = expanded_selection(layout, selection);
    let Some(anchor) = layout
        .nodes()
        .iter()
        .filter(|node| selected.contains(&node.id()))
        .min_by_key(|node| node.id())
    else {
        return layout.clone();
    };
    let snapped_delta_x = snap(anchor.position().x() + delta_x) - anchor.position().x();
    let snapped_delta_y = snap(anchor.position().y() + delta_y) - anchor.position().y();
    map_nodes(layout, |node| {
        if selected.contains(&node.id()) {
            with_geometry(
                node,
                CanvasPoint::new(
                    node.position().x() + snapped_delta_x,
                    node.position().y() + snapped_delta_y,
                )
                .expect("finite canvas movement remains finite"),
                node.size(),
            )
        } else {
            node.clone()
        }
    })
}

pub(crate) fn resize_node(
    layout: &CanvasLayout,
    node_id: NodeId,
    width: f32,
    height: f32,
) -> CanvasLayout {
    map_nodes(layout, |node| {
        if node.id() == node_id {
            with_geometry(
                node,
                node.position(),
                CanvasSize::new(
                    snap(width).max(MIN_NODE_WIDTH),
                    snap(height).max(MIN_NODE_HEIGHT),
                )
                .expect("minimum node dimensions are positive and finite"),
            )
        } else {
            node.clone()
        }
    })
}

pub(crate) fn duplicate(
    layout: &CanvasLayout,
    selection: &[NodeId],
    all: &CanvasLayout,
) -> (CanvasLayout, Vec<NodeId>) {
    let selected = expanded_selection(layout, selection);
    if selected.is_empty() {
        return (layout.clone(), Vec::new());
    }
    let mut next_node = next_id(all.nodes().iter().map(|node| node.id().get()));
    let mut next_group = next_id(all.groups().iter().map(|group| group.id().get()));
    let mut next_connection = next_id(
        all.connections()
            .iter()
            .map(|connection| connection.id().get()),
    );
    let mut next_z = layout
        .nodes()
        .iter()
        .map(Node::z_index)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let mut ids = BTreeMap::new();
    let mut nodes = layout.nodes().to_vec();
    for node in layout
        .nodes()
        .iter()
        .filter(|node| selected.contains(&node.id()))
    {
        let id = NodeId::new(next_node);
        next_node = next_node.saturating_add(1);
        ids.insert(node.id(), id);
        nodes.push(Node::with_content_and_z_index(
            id,
            node.content().clone(),
            CanvasPoint::new(
                node.position().x() + DUPLICATE_OFFSET,
                node.position().y() + DUPLICATE_OFFSET,
            )
            .expect("finite duplicate offset remains finite"),
            node.size(),
            next_z,
        ));
        next_z = next_z.saturating_add(1);
    }

    let mut groups = layout.groups().to_vec();
    for group in layout.groups() {
        let members = group
            .members()
            .map(|member| ids.get(&member).copied())
            .collect::<Option<Vec<_>>>();
        if let Some(members) = members {
            groups.push(NodeGroup::new(NodeGroupId::new(next_group), members));
            next_group = next_group.saturating_add(1);
        }
    }

    let mut connections = layout.connections().to_vec();
    for connection in layout.connections() {
        if let (Some(source), Some(target)) =
            (ids.get(&connection.source()), ids.get(&connection.target()))
        {
            connections.push(Connection::new(
                ConnectionId::new(next_connection),
                *source,
                *target,
                connection.kind(),
            ));
            next_connection = next_connection.saturating_add(1);
        }
    }

    let selection = ids.values().copied().collect();
    (CanvasLayout::new(nodes, groups, connections), selection)
}

pub(crate) fn selection_fragment(layout: &CanvasLayout, selection: &[NodeId]) -> CanvasLayout {
    let selected = expanded_selection(layout, selection);
    CanvasLayout::new(
        layout
            .nodes()
            .iter()
            .filter(|node| selected.contains(&node.id()))
            .cloned()
            .collect(),
        layout
            .groups()
            .iter()
            .filter(|group| group.members().all(|member| selected.contains(&member)))
            .cloned()
            .collect(),
        layout
            .connections()
            .iter()
            .filter(|connection| {
                selected.contains(&connection.source()) && selected.contains(&connection.target())
            })
            .cloned()
            .collect(),
    )
}

pub(crate) fn paste_fragment(
    layout: &CanvasLayout,
    fragment: &CanvasLayout,
    all: &CanvasLayout,
) -> (CanvasLayout, Vec<NodeId>) {
    if fragment.nodes().is_empty() {
        return (layout.clone(), Vec::new());
    }
    let mut next_node = next_id(all.nodes().iter().map(|node| node.id().get()));
    let mut next_group = next_id(all.groups().iter().map(|group| group.id().get()));
    let mut next_connection = next_id(
        all.connections()
            .iter()
            .map(|connection| connection.id().get()),
    );
    let mut next_z = layout
        .nodes()
        .iter()
        .map(Node::z_index)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let mut ids = BTreeMap::new();
    let mut nodes = layout.nodes().to_vec();
    for node in fragment.nodes() {
        let id = NodeId::new(next_node);
        next_node = next_node.saturating_add(1);
        ids.insert(node.id(), id);
        nodes.push(Node::with_content_and_z_index(
            id,
            node.content().clone(),
            CanvasPoint::new(
                node.position().x() + DUPLICATE_OFFSET,
                node.position().y() + DUPLICATE_OFFSET,
            )
            .expect("finite paste offset remains finite"),
            node.size(),
            next_z,
        ));
        next_z = next_z.saturating_add(1);
    }

    let mut groups = layout.groups().to_vec();
    for group in fragment.groups() {
        let members = group
            .members()
            .map(|member| ids.get(&member).copied())
            .collect::<Option<Vec<_>>>();
        if let Some(members) = members {
            groups.push(NodeGroup::new(NodeGroupId::new(next_group), members));
            next_group = next_group.saturating_add(1);
        }
    }

    let mut connections = layout.connections().to_vec();
    for connection in fragment.connections() {
        if let (Some(source), Some(target)) =
            (ids.get(&connection.source()), ids.get(&connection.target()))
        {
            connections.push(Connection::new(
                ConnectionId::new(next_connection),
                *source,
                *target,
                connection.kind(),
            ));
            next_connection = next_connection.saturating_add(1);
        }
    }

    (
        CanvasLayout::new(nodes, groups, connections),
        ids.values().copied().collect(),
    )
}

pub(crate) fn remove(layout: &CanvasLayout, selection: &[NodeId]) -> CanvasLayout {
    let selected = selection.iter().copied().collect::<BTreeSet<_>>();
    CanvasLayout::new(
        layout
            .nodes()
            .iter()
            .filter(|node| !selected.contains(&node.id()))
            .cloned()
            .collect(),
        layout
            .groups()
            .iter()
            .filter_map(|group| {
                let members = group
                    .members()
                    .filter(|node_id| !selected.contains(node_id))
                    .collect::<Vec<_>>();
                (members.len() >= 2).then(|| NodeGroup::new(group.id(), members))
            })
            .collect(),
        layout
            .connections()
            .iter()
            .filter(|connection| {
                !selected.contains(&connection.source()) && !selected.contains(&connection.target())
            })
            .cloned()
            .collect(),
    )
}

pub(crate) fn group(
    layout: &CanvasLayout,
    selection: &[NodeId],
    all: &CanvasLayout,
) -> CanvasLayout {
    let selected = selection.iter().copied().collect::<BTreeSet<_>>();
    if selected.len() < 2 {
        return layout.clone();
    }
    let mut groups = layout
        .groups()
        .iter()
        .filter_map(|group| {
            let members = group
                .members()
                .filter(|member| !selected.contains(member))
                .collect::<Vec<_>>();
            (members.len() >= 2).then(|| NodeGroup::new(group.id(), members))
        })
        .collect::<Vec<_>>();
    groups.push(NodeGroup::new(
        NodeGroupId::new(next_id(all.groups().iter().map(|group| group.id().get()))),
        selected,
    ));
    CanvasLayout::new(
        layout.nodes().to_vec(),
        groups,
        layout.connections().to_vec(),
    )
}

pub(crate) fn ungroup(layout: &CanvasLayout, selection: &[NodeId]) -> CanvasLayout {
    let selected = selection.iter().copied().collect::<BTreeSet<_>>();
    CanvasLayout::new(
        layout.nodes().to_vec(),
        layout
            .groups()
            .iter()
            .filter(|group| !group.members().any(|member| selected.contains(&member)))
            .cloned()
            .collect(),
        layout.connections().to_vec(),
    )
}

pub(crate) fn connect(
    layout: &CanvasLayout,
    selection: &[NodeId],
    all: &CanvasLayout,
) -> Result<CanvasLayout, &'static str> {
    if selection.len() != 2 {
        return Err("select exactly two nodes to connect");
    }
    let nodes = layout
        .nodes()
        .iter()
        .map(|node| (node.id(), node))
        .collect::<BTreeMap<_, _>>();
    let source = nodes
        .get(&selection[0])
        .ok_or("the source node no longer exists")?;
    let target = nodes
        .get(&selection[1])
        .ok_or("the target node no longer exists")?;
    if layout
        .connections()
        .iter()
        .any(|connection| connection.source() == source.id() && connection.target() == target.id())
    {
        return Err("these nodes are already connected in that direction");
    }
    let kind = ConnectionKind::between_content(source.content(), target.content())
        .ok_or("the selected node types cannot be connected in that direction")?;
    let mut connections = layout.connections().to_vec();
    connections.push(Connection::new(
        ConnectionId::new(next_id(
            all.connections()
                .iter()
                .map(|connection| connection.id().get()),
        )),
        source.id(),
        target.id(),
        kind,
    ));
    Ok(CanvasLayout::new(
        layout.nodes().to_vec(),
        layout.groups().to_vec(),
        connections,
    ))
}

pub(crate) fn align(
    layout: &CanvasLayout,
    selection: &[NodeId],
    alignment: Alignment,
) -> CanvasLayout {
    let selected = expanded_selection(layout, selection);
    let nodes = layout
        .nodes()
        .iter()
        .filter(|node| selected.contains(&node.id()))
        .collect::<Vec<_>>();
    if nodes.len() < 2 {
        return layout.clone();
    }

    let positions = match alignment {
        Alignment::HorizontalCenters => {
            let center = nodes.iter().map(|node| center_x(node)).sum::<f32>() / nodes.len() as f32;
            nodes
                .iter()
                .map(|node| {
                    (
                        node.id(),
                        snap(center - node.size().width() / 2.0),
                        node.position().y(),
                    )
                })
                .collect()
        }
        Alignment::VerticalCenters => {
            let center = nodes.iter().map(|node| center_y(node)).sum::<f32>() / nodes.len() as f32;
            nodes
                .iter()
                .map(|node| {
                    (
                        node.id(),
                        node.position().x(),
                        snap(center - node.size().height() / 2.0),
                    )
                })
                .collect()
        }
        Alignment::DistributeHorizontally => distribute(&nodes, true),
        Alignment::DistributeVertically => distribute(&nodes, false),
    };
    let positions = positions
        .into_iter()
        .map(|(node_id, x, y)| (node_id, (x, y)))
        .collect::<BTreeMap<_, _>>();
    map_nodes(layout, |node| {
        positions.get(&node.id()).map_or_else(
            || node.clone(),
            |(x, y)| {
                with_geometry(
                    node,
                    CanvasPoint::new(*x, *y).expect("aligned positions remain finite"),
                    node.size(),
                )
            },
        )
    })
}

pub(crate) fn change_z_order(
    layout: &CanvasLayout,
    selection: &[NodeId],
    order: ZOrder,
) -> CanvasLayout {
    let selected = expanded_selection(layout, selection);
    if selected.is_empty() {
        return layout.clone();
    }
    let mut ordered = layout.nodes().iter().collect::<Vec<_>>();
    ordered.sort_by_key(|node| (node.z_index(), node.id()));
    ordered.sort_by_key(|node| match order {
        ZOrder::Front => selected.contains(&node.id()),
        ZOrder::Back => !selected.contains(&node.id()),
    });
    let z_indexes = ordered
        .into_iter()
        .enumerate()
        .map(|(index, node)| (node.id(), i32::try_from(index).unwrap_or(i32::MAX)))
        .collect::<BTreeMap<_, _>>();
    map_nodes(layout, |node| {
        Node::with_content_and_z_index(
            node.id(),
            node.content().clone(),
            node.position(),
            node.size(),
            z_indexes[&node.id()],
        )
    })
}

fn map_nodes(layout: &CanvasLayout, map: impl Fn(&Node) -> Node) -> CanvasLayout {
    CanvasLayout::new(
        layout.nodes().iter().map(map).collect(),
        layout.groups().to_vec(),
        layout.connections().to_vec(),
    )
}

fn with_geometry(node: &Node, position: CanvasPoint, size: CanvasSize) -> Node {
    Node::with_content_and_z_index(
        node.id(),
        node.content().clone(),
        position,
        size,
        node.z_index(),
    )
}

fn group_members(layout: &CanvasLayout, node_id: NodeId) -> Vec<NodeId> {
    layout
        .groups()
        .iter()
        .find(|group| group.contains(node_id))
        .map_or_else(|| vec![node_id], |group| group.members().collect())
}

fn expanded_selection(layout: &CanvasLayout, selection: &[NodeId]) -> BTreeSet<NodeId> {
    let mut expanded = selection.iter().copied().collect::<BTreeSet<_>>();
    for group in layout.groups() {
        if group.members().any(|member| expanded.contains(&member)) {
            expanded.extend(group.members());
        }
    }
    expanded
}

fn distribute(nodes: &[&Node], horizontal: bool) -> Vec<(NodeId, f32, f32)> {
    if nodes.len() < 3 {
        return Vec::new();
    }
    let mut ordered = nodes.to_vec();
    ordered.sort_by(|left, right| {
        let left = if horizontal {
            center_x(left)
        } else {
            center_y(left)
        };
        let right = if horizontal {
            center_x(right)
        } else {
            center_y(right)
        };
        left.total_cmp(&right)
    });
    let first = if horizontal {
        center_x(ordered[0])
    } else {
        center_y(ordered[0])
    };
    let last = if horizontal {
        center_x(ordered[ordered.len() - 1])
    } else {
        center_y(ordered[ordered.len() - 1])
    };
    let step = (last - first) / (ordered.len() - 1) as f32;
    ordered
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let center = first + step * index as f32;
            if horizontal {
                (
                    node.id(),
                    snap(center - node.size().width() / 2.0),
                    node.position().y(),
                )
            } else {
                (
                    node.id(),
                    node.position().x(),
                    snap(center - node.size().height() / 2.0),
                )
            }
        })
        .collect()
}

fn center_x(node: &Node) -> f32 {
    node.position().x() + node.size().width() / 2.0
}

fn center_y(node: &Node) -> f32 {
    node.position().y() + node.size().height() / 2.0
}

fn snap(value: f32) -> f32 {
    (value / GRID_STEP).round() * GRID_STEP
}

fn next_id(ids: impl Iterator<Item = u64>) -> u64 {
    ids.max().unwrap_or(0).saturating_add(1)
}

#[cfg(test)]
mod tests {
    use openpodium::domain::{AgentId, NodeTarget};

    use super::*;

    #[test]
    fn duplicated_nodes_use_ids_above_nodes_on_inactive_floors() {
        let first = Node::new(
            NodeId::new(1),
            NodeTarget::Agent(AgentId::new(1)),
            CanvasPoint::new(0.0, 0.0).unwrap(),
            CanvasSize::new(300.0, 200.0).unwrap(),
        );
        let hidden = Node::new(
            NodeId::new(10),
            NodeTarget::Agent(AgentId::new(2)),
            CanvasPoint::new(0.0, 0.0).unwrap(),
            CanvasSize::new(300.0, 200.0).unwrap(),
        );
        let visible = CanvasLayout::new(vec![first.clone()], vec![], vec![]);
        let all = CanvasLayout::new(vec![first, hidden], vec![], vec![]);
        let (_, selection) = duplicate(&visible, &[NodeId::new(1)], &all);
        assert_eq!(selection, vec![NodeId::new(11)]);
    }

    #[test]
    fn grouped_nodes_move_together_and_snap_once() {
        let layout = layout();
        let moved = move_nodes(&layout, &[NodeId::new(1)], 31.0, 49.0);

        assert_eq!(moved.nodes()[0].position().x(), 40.0);
        assert_eq!(moved.nodes()[0].position().y(), 40.0);
        assert_eq!(moved.nodes()[1].position().x(), 440.0);
        assert_eq!(moved.nodes()[1].position().y(), 40.0);
    }

    #[test]
    fn removing_a_node_cleans_groups_and_connections_atomically() {
        let removed = remove(&layout(), &[NodeId::new(1)]);

        assert_eq!(removed.nodes().len(), 1);
        assert_eq!(removed.nodes()[0].id(), NodeId::new(2));
        assert!(removed.groups().is_empty());
        assert!(removed.connections().is_empty());
    }

    #[test]
    fn duplication_preserves_internal_relationships() {
        let layout = layout();
        let (duplicated, selection) = duplicate(&layout, &[NodeId::new(1)], &layout);

        assert_eq!(selection, vec![NodeId::new(3), NodeId::new(4)]);
        assert_eq!(duplicated.nodes().len(), 4);
        assert_eq!(duplicated.groups().len(), 2);
        assert_eq!(duplicated.connections().len(), 2);
        assert_eq!(duplicated.connections()[1].source(), NodeId::new(3));
        assert_eq!(duplicated.connections()[1].target(), NodeId::new(4));
    }

    #[test]
    fn history_moves_snapshots_between_undo_and_redo() {
        let before = layout();
        let after = move_nodes(&before, &[NodeId::new(1)], 20.0, 0.0);
        let mut history = History::default();
        history.record(before.clone());

        assert_eq!(history.undo_target(), Some(&before));
        history.complete_undo(after.clone());
        assert_eq!(history.redo_target(), Some(&after));
        history.complete_redo(before);
        assert!(history.can_undo());
        assert!(!history.can_redo());
    }

    #[test]
    fn portable_fragments_remap_owned_nodes_and_internal_connections() {
        use openpodium::domain::{CanvasNodeContent, CanvasText};

        let first = Node::with_content(
            NodeId::new(10),
            CanvasNodeContent::Text {
                markdown: CanvasText::new("First").unwrap(),
            },
            CanvasPoint::new(0.0, 0.0).unwrap(),
            CanvasSize::new(240.0, 160.0).unwrap(),
        );
        let second = Node::with_content(
            NodeId::new(11),
            CanvasNodeContent::Text {
                markdown: CanvasText::new("Second").unwrap(),
            },
            CanvasPoint::new(300.0, 0.0).unwrap(),
            CanvasSize::new(240.0, 160.0).unwrap(),
        );
        let source = CanvasLayout::new(
            vec![first, second],
            vec![],
            vec![Connection::new(
                ConnectionId::new(4),
                NodeId::new(10),
                NodeId::new(11),
                ConnectionKind::Reference,
            )],
        );
        let fragment = selection_fragment(&source, &[NodeId::new(10), NodeId::new(11)]);
        let (pasted, selection) = paste_fragment(&CanvasLayout::default(), &fragment, &source);

        assert_eq!(selection, vec![NodeId::new(12), NodeId::new(13)]);
        assert_eq!(pasted.nodes()[0].content(), source.nodes()[0].content());
        assert_eq!(pasted.connections()[0].source(), NodeId::new(12));
        assert_eq!(pasted.connections()[0].target(), NodeId::new(13));
        assert_eq!(pasted.connections()[0].kind(), ConnectionKind::Reference);
    }

    #[test]
    fn multi_selection_order_defines_connection_direction() {
        let layout = CanvasLayout::new(vec![node(1, 0.0), node(2, 400.0)], vec![], vec![]);
        let selection = selection_for_click(&layout, &[], NodeId::new(2), false);
        let selection = selection_for_click(&layout, &selection, NodeId::new(1), true);
        let connected = connect(&layout, &selection, &layout).unwrap();

        assert_eq!(selection, vec![NodeId::new(2), NodeId::new(1)]);
        assert_eq!(connected.connections()[0].source(), NodeId::new(2));
        assert_eq!(connected.connections()[0].target(), NodeId::new(1));
        assert_eq!(
            connected.connections()[0].kind(),
            ConnectionKind::Coordination
        );
    }

    #[test]
    fn resizing_snaps_and_respects_minimum_dimensions() {
        let layout = CanvasLayout::new(vec![node(1, 0.0)], vec![], vec![]);
        let minimum = resize_node(&layout, NodeId::new(1), 211.0, 131.0);
        let snapped = resize_node(&layout, NodeId::new(1), 333.0, 247.0);

        assert_eq!(minimum.nodes()[0].size().width(), MIN_NODE_WIDTH);
        assert_eq!(minimum.nodes()[0].size().height(), MIN_NODE_HEIGHT);
        assert_eq!(snapped.nodes()[0].size().width(), 340.0);
        assert_eq!(snapped.nodes()[0].size().height(), 240.0);
    }

    #[test]
    fn grouping_and_z_order_are_deterministic() {
        let layout = CanvasLayout::new(
            vec![node(1, 0.0), node(2, 400.0), node(3, 800.0)],
            vec![],
            vec![],
        );
        let grouped = group(&layout, &[NodeId::new(1), NodeId::new(2)], &layout);
        let front = change_z_order(&grouped, &[NodeId::new(1)], ZOrder::Front);

        assert_eq!(grouped.groups().len(), 1);
        assert_eq!(grouped.groups()[0].len(), 2);
        assert!(front.nodes()[2].z_index() < front.nodes()[0].z_index());
        assert!(front.nodes()[2].z_index() < front.nodes()[1].z_index());
        assert!(ungroup(&grouped, &[NodeId::new(1)]).groups().is_empty());
    }

    fn layout() -> CanvasLayout {
        CanvasLayout::new(
            vec![node(1, 0.0), node(2, 400.0)],
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
        )
    }

    fn node(id: u64, x: f32) -> Node {
        Node::new(
            NodeId::new(id),
            NodeTarget::Agent(AgentId::new(id)),
            CanvasPoint::new(x, 0.0).unwrap(),
            CanvasSize::new(320.0, 240.0).unwrap(),
        )
    }
}
