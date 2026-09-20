use std::collections::BTreeMap;

use super::{AgentId, Name, NodeId, TaskId, WorkspaceDirectory};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloorOwner {
    Agent(AgentId),
    Task(TaskId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloorLifecycle {
    Available,
    Missing,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Floor {
    pub name: Name,
    pub directory: WorkspaceDirectory,
    pub repository: WorkspaceDirectory,
    pub branch: Option<String>,
    pub base_revision: String,
    pub base_branch: Option<String>,
    pub managed: bool,
    pub ownership_token: Option<String>,
    pub owner: Option<FloorOwner>,
    pub dirty: bool,
    pub lifecycle: FloorLifecycle,
}

/// Nodes without an entry belong to the original checkout (the main floor).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Floors {
    pub entries: BTreeMap<u64, Floor>,
    pub node_floors: BTreeMap<NodeId, u64>,
    pub active: Option<u64>,
}

impl Floors {
    pub fn contains_node(&self, node: NodeId) -> bool {
        self.node_floors.get(&node).copied() == self.active
    }
}
