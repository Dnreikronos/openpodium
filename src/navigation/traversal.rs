use std::cmp::Ordering;
use std::collections::BTreeSet;

use crate::domain::{CanvasLayout, ConnectionKind, Node, NodeId};

pub fn traverse_nodes(
    layout: &CanvasLayout,
    current: Option<NodeId>,
    forward: bool,
) -> Option<NodeId> {
    let mut nodes: Vec<_> = layout.nodes().iter().collect();
    nodes.sort_by(compare_nodes);
    cycle(nodes.into_iter().map(Node::id).collect(), current, forward)
}

pub fn traverse_connections(
    layout: &CanvasLayout,
    current: NodeId,
    selected_neighbor: Option<NodeId>,
    forward: bool,
) -> Option<NodeId> {
    let mut outgoing = Vec::new();
    let mut incoming = Vec::new();
    for connection in layout.connections() {
        if connection.source() == current {
            outgoing.push((connection_kind_rank(connection.kind()), connection.target()));
        } else if connection.target() == current {
            incoming.push((connection_kind_rank(connection.kind()), connection.source()));
        }
    }
    outgoing.sort_unstable();
    incoming.sort_unstable();
    let mut seen = BTreeSet::new();
    let neighbors = outgoing
        .into_iter()
        .chain(incoming)
        .map(|(_, node)| node)
        .filter(|node| seen.insert(*node))
        .collect();
    cycle(neighbors, selected_neighbor, forward)
}

fn compare_nodes(left: &&Node, right: &&Node) -> Ordering {
    left.position()
        .y()
        .total_cmp(&right.position().y())
        .then_with(|| left.position().x().total_cmp(&right.position().x()))
        .then_with(|| left.id().cmp(&right.id()))
}

fn connection_kind_rank(kind: ConnectionKind) -> u8 {
    match kind {
        ConnectionKind::Coordination => 0,
        ConnectionKind::Assignment => 1,
        ConnectionKind::Dependency => 2,
        ConnectionKind::Handoff => 3,
        ConnectionKind::Reference => 4,
    }
}

fn cycle(values: Vec<NodeId>, current: Option<NodeId>, forward: bool) -> Option<NodeId> {
    if values.is_empty() {
        return None;
    }
    let index = current.and_then(|current| values.iter().position(|value| *value == current));
    Some(match (index, forward) {
        (Some(index), true) => values[(index + 1) % values.len()],
        (Some(0), false) | (None, false) => *values.last().expect("values is not empty"),
        (Some(index), false) => values[index - 1],
        (None, true) => values[0],
    })
}

#[cfg(test)]
mod tests {
    use crate::domain::{CanvasPoint, CanvasSize, Connection, ConnectionId, NodeTarget};

    use super::*;

    fn node(id: u64, x: f32, y: f32) -> Node {
        Node::new(
            NodeId::new(id),
            NodeTarget::Agent(crate::domain::AgentId::new(id)),
            CanvasPoint::new(x, y).unwrap(),
            CanvasSize::new(100.0, 100.0).unwrap(),
        )
    }

    #[test]
    fn spatial_traversal_is_stable_and_wraps() {
        let layout = CanvasLayout::new(
            vec![node(3, 0.0, 20.0), node(2, 20.0, 0.0), node(1, 0.0, 0.0)],
            vec![],
            vec![],
        );
        assert_eq!(traverse_nodes(&layout, None, true), Some(NodeId::new(1)));
        assert_eq!(
            traverse_nodes(&layout, Some(NodeId::new(1)), true),
            Some(NodeId::new(2))
        );
        assert_eq!(
            traverse_nodes(&layout, Some(NodeId::new(3)), true),
            Some(NodeId::new(1))
        );
        assert_eq!(
            traverse_nodes(&layout, Some(NodeId::new(1)), false),
            Some(NodeId::new(3))
        );
    }

    #[test]
    fn connection_traversal_orders_outgoing_before_incoming_and_wraps() {
        let layout = CanvasLayout::new(
            vec![node(1, 0.0, 0.0), node(2, 0.0, 0.0), node(3, 0.0, 0.0)],
            vec![],
            vec![
                Connection::new(
                    ConnectionId::new(1),
                    NodeId::new(3),
                    NodeId::new(1),
                    ConnectionKind::Reference,
                ),
                Connection::new(
                    ConnectionId::new(2),
                    NodeId::new(1),
                    NodeId::new(2),
                    ConnectionKind::Reference,
                ),
            ],
        );
        assert_eq!(
            traverse_connections(&layout, NodeId::new(1), None, true),
            Some(NodeId::new(2))
        );
        assert_eq!(
            traverse_connections(&layout, NodeId::new(1), Some(NodeId::new(2)), true),
            Some(NodeId::new(3))
        );
    }
}
