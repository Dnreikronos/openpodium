use std::path::Path;

use crate::domain::{
    DomainCommand, Floor, FloorLifecycle, FloorOwner, Floors, Name, NodeTarget, Timestamp,
    WorkspaceDirectory, WorkspaceId,
};
use crate::git::{Checkout, Repository, validate_floor_name, validate_name};

use super::WorkspaceManager;

#[derive(Debug, Clone)]
pub enum FloorOperation {
    Refresh,
    Create {
        name: String,
        branch: String,
        owner: Option<FloorOwner>,
    },
    Remove {
        floor: u64,
        discard: bool,
    },
}

pub struct FloorJob {
    workspace: crate::domain::Workspace,
    parent: std::path::PathBuf,
    operation: FloorOperation,
}

#[derive(Debug, Clone)]
pub struct FloorResult {
    workspace: WorkspaceId,
    directory: Option<WorkspaceDirectory>,
    before: Floors,
    after: Floors,
}

impl FloorJob {
    pub fn run(self) -> Result<FloorResult, String> {
        let after = match self.operation {
            FloorOperation::Refresh => refresh(&self.workspace)?,
            FloorOperation::Create {
                name,
                branch,
                owner,
            } => create(&self.workspace, &name, &branch, owner, &self.parent)?,
            FloorOperation::Remove { floor, discard } => remove(&self.workspace, floor, discard)?,
        };
        Ok(FloorResult {
            workspace: self.workspace.id(),
            directory: self.workspace.settings().working_directory().cloned(),
            before: self.workspace.floors().clone(),
            after,
        })
    }
}

impl WorkspaceManager {
    pub fn prepare_floor_operation(
        &self,
        id: WorkspaceId,
        operation: FloorOperation,
    ) -> Result<FloorJob, String> {
        let workspace = self.workspace(id).ok_or("Unknown workspace")?.clone();
        let parent = self
            .journal
            .path()
            .parent()
            .ok_or("Workspace database has no parent directory")?
            .join("worktrees")
            .join(id.get().to_string());
        Ok(FloorJob {
            workspace,
            parent,
            operation,
        })
    }

    pub fn finish_floor_operation(
        &mut self,
        result: FloorResult,
        at: Timestamp,
    ) -> Result<(), String> {
        let workspace = self
            .workspace(result.workspace)
            .ok_or("Unknown workspace")?;
        let before = workspace.floors().clone();
        if before.entries != result.before.entries
            || workspace.settings().working_directory() != result.directory.as_ref()
        {
            return Err("The workspace changed during the Git operation. Refresh its worktrees to reconcile; no checkout will be automatically claimed or deleted.".to_owned());
        }
        let mut after = result.after;
        after.node_floors = before.node_floors.clone();
        if before.active != result.before.active {
            after.active = before.active;
        }
        self.save_floors(result.workspace, before, after, at)
            .map_err(|e| format!("Git completed, but floor state could not be saved: {e}. Refresh worktrees to reconcile; any unrecorded checkout remains user-owned."))
    }

    pub fn refresh_floors(&mut self, id: WorkspaceId, at: Timestamp) -> Result<(), String> {
        let result = self
            .prepare_floor_operation(id, FloorOperation::Refresh)?
            .run()?;
        self.finish_floor_operation(result, at)
    }

    pub fn create_floor(
        &mut self,
        id: WorkspaceId,
        name: &str,
        branch: &str,
        owner: Option<FloorOwner>,
        at: Timestamp,
    ) -> Result<u64, String> {
        let result = self
            .prepare_floor_operation(
                id,
                FloorOperation::Create {
                    name: name.to_owned(),
                    branch: branch.to_owned(),
                    owner,
                },
            )?
            .run()?;
        let floor = result.after.active.ok_or("Created floor is missing")?;
        self.finish_floor_operation(result, at)?;
        Ok(floor)
    }

    pub fn remove_floor(
        &mut self,
        id: WorkspaceId,
        floor: u64,
        discard: bool,
        at: Timestamp,
    ) -> Result<(), String> {
        let result = self
            .prepare_floor_operation(id, FloorOperation::Remove { floor, discard })?
            .run()?;
        self.finish_floor_operation(result, at)
    }

    pub fn switch_floor(
        &mut self,
        id: WorkspaceId,
        floor: Option<u64>,
        at: Timestamp,
    ) -> Result<(), String> {
        let before = self
            .workspace(id)
            .ok_or("Unknown workspace")?
            .floors()
            .clone();
        if floor.is_some_and(|id| !before.entries.contains_key(&id)) {
            return Err("Unknown floor".to_owned());
        }
        let mut after = before.clone();
        after.active = floor;
        self.save_floors(id, before, after, at)
    }

    fn save_floors(
        &mut self,
        id: WorkspaceId,
        before: Floors,
        after: Floors,
        at: Timestamp,
    ) -> Result<(), String> {
        if before != after {
            self.execute(id, DomainCommand::ReplaceFloors { before, after }, at)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn refresh(workspace: &crate::domain::Workspace) -> Result<Floors, String> {
    let repo = floor_repository(workspace)?;
    let before = workspace.floors().clone();
    let mut after = before.clone();
    let checkouts = repo.checkouts()?;
    for floor in after.entries.values_mut() {
        if floor.lifecycle != FloorLifecycle::Removed {
            floor.lifecycle = FloorLifecycle::Missing;
        }
    }
    for checkout in checkouts {
        if checkout.path == repo.root {
            continue;
        }
        let path = encoded(&checkout.path)?;
        let existing = after.entries.values_mut().find(|f| {
            same_path(Path::new(f.directory.as_str()), &checkout.path)
                && f.lifecycle != FloorLifecycle::Removed
        });
        if let Some(floor) = existing {
            // A switched branch or replaced repository never inherits ownership.
            if floor.branch != checkout.branch
                || (floor.managed
                    && checkout.path.is_dir()
                    && !repo.owns(&checkout, floor.ownership_token.as_deref())?)
                || !same_path(Path::new(floor.repository.as_str()), &repo.common_directory)
            {
                floor.managed = false;
                floor.ownership_token = None;
                floor.owner = None;
            }
            floor.branch = checkout.branch.clone();
            floor.lifecycle = if checkout.path.is_dir() {
                FloorLifecycle::Available
            } else {
                FloorLifecycle::Missing
            };
            if floor.lifecycle == FloorLifecycle::Available {
                floor.dirty = repo.dirty(&checkout)?;
            }
        } else {
            let id = next_id(&after)?;
            let name = checkout
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Worktree");
            after.entries.insert(
                id,
                Floor {
                    name: Name::new(name).map_err(|e| e.to_string())?,
                    directory: path,
                    repository: encoded(&repo.common_directory)?,
                    branch: checkout.branch.clone(),
                    base_revision: checkout.head.clone(),
                    base_branch: None,
                    managed: false,
                    ownership_token: None,
                    owner: None,
                    dirty: if checkout.path.is_dir() {
                        repo.dirty(&checkout)?
                    } else {
                        false
                    },
                    lifecycle: if checkout.path.is_dir() {
                        FloorLifecycle::Available
                    } else {
                        FloorLifecycle::Missing
                    },
                },
            );
        }
    }
    Ok(after)
}

fn create(
    workspace: &crate::domain::Workspace,
    name: &str,
    branch: &str,
    owner: Option<FloorOwner>,
    parent: &Path,
) -> Result<Floors, String> {
    validate_floor_name(name)?;
    validate_name(branch)?;
    if let Some(owner) = owner {
        let exists = match owner {
            FloorOwner::Agent(id) => workspace.agent(id).is_some(),
            FloorOwner::Task(id) => workspace.task(id).is_some(),
        };
        if !exists {
            return Err("The floor owner does not exist in this workspace".to_owned());
        }
    }
    let before = workspace.floors().clone();
    let floor_id = next_id(&before)?;
    let repo = floor_repository(workspace)?;
    let base = repo
        .checkouts()?
        .into_iter()
        .find(|c| c.path == repo.root)
        .ok_or("Original checkout is missing")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let checkout = repo.create(name, branch, parent)?;
    let ownership_token = repo.claim(&checkout)?;
    let mut after = before.clone();
    after.entries.insert(
        floor_id,
        Floor {
            name: Name::new(name).map_err(|e| e.to_string())?,
            directory: encoded(&checkout.path)?,
            repository: encoded(&repo.common_directory)?,
            branch: checkout.branch,
            base_revision: checkout.head,
            base_branch: base.branch,
            managed: true,
            ownership_token: Some(ownership_token),
            owner,
            dirty: false,
            lifecycle: FloorLifecycle::Available,
        },
    );
    after.active = Some(floor_id);
    Ok(after)
}

fn remove(
    workspace: &crate::domain::Workspace,
    floor_id: u64,
    discard: bool,
) -> Result<Floors, String> {
    let before = workspace.floors().clone();
    let floor = before.entries.get(&floor_id).ok_or("Unknown floor")?;
    if !floor.managed || floor.lifecycle != FloorLifecycle::Available {
        return Err("Only available, managed floors can be removed".to_owned());
    }
    for (node, assigned) in &before.node_floors {
        if *assigned == floor_id
            && let Some(node) = workspace.node(*node)
            && let NodeTarget::Agent(agent_id) = node.target()
            && let Some(agent) = workspace.agent(agent_id)
            && matches!(
                agent.state(),
                crate::domain::AgentState::Starting
                    | crate::domain::AgentState::Running
                    | crate::domain::AgentState::Waiting
            )
        {
            return Err("Stop all agents on this floor before cleanup".to_owned());
        }
    }
    let repo = floor_repository(workspace)?;
    if !same_path(&repo.common_directory, Path::new(floor.repository.as_str())) {
        return Err("The workspace repository changed".to_owned());
    }
    let checkout = Checkout {
        path: floor.directory.as_str().into(),
        head: floor.base_revision.clone(),
        branch: floor.branch.clone(),
        locked: false,
    };
    if !repo.owns(&checkout, floor.ownership_token.as_deref())? {
        return Err("Checkout ownership changed; refresh before continuing".to_owned());
    }
    let base = floor
        .base_branch
        .as_deref()
        .ok_or("This floor was created from a detached checkout; integrate and clean up in Git")?;
    repo.remove(&checkout, floor.managed, base, discard)?;
    let mut after = before.clone();
    after
        .entries
        .get_mut(&floor_id)
        .expect("floor was checked")
        .lifecycle = FloorLifecycle::Removed;
    if after.active == Some(floor_id) {
        after.active = None;
    }
    Ok(after)
}

fn floor_repository(workspace: &crate::domain::Workspace) -> Result<Repository, String> {
    let directory = workspace
        .settings()
        .working_directory()
        .ok_or("Workspace has no working directory")?;
    Repository::discover(Path::new(directory.as_str()))
}

fn next_id(floors: &Floors) -> Result<u64, String> {
    floors
        .entries
        .keys()
        .last()
        .copied()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| "Floor identifiers exhausted".to_owned())
}

fn encoded(path: &Path) -> Result<WorkspaceDirectory, String> {
    WorkspaceDirectory::new(
        path.to_str()
            .ok_or("Worktree paths must be valid Unicode")?,
    )
    .map_err(|e| e.to_string())
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (dunce::canonicalize(left), dunce::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => dunce::simplified(left) == dunce::simplified(right),
    }
}

#[cfg(test)]
mod tests;
