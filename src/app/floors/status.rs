use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use iced::widget::{column, text};
use iced::{Element, Task};
use openpodium::domain::NodeId;
use openpodium::git::{ChangeKind, ChangedPath, CollisionReport, CollisionSeverity, Repository};

use super::super::{Message as AppMessage, OpenPodium, WorkspaceManager};

#[derive(Default)]
pub(super) struct UiState {
    busy: bool,
    last_poll: Option<Instant>,
    signature: Option<String>,
    main: Option<PathBuf>,
    pub(super) report: CollisionReport,
}

#[derive(Clone)]
pub(crate) enum Message {
    Completed(openpodium::domain::WorkspaceId, Result<ScanResult, String>),
}

#[derive(Clone)]
pub(crate) struct ScanResult {
    signature: String,
    main: PathBuf,
    report: Option<CollisionReport>,
}

pub(super) fn scan(state: &mut OpenPodium, force: bool) -> Task<AppMessage> {
    if state.floor_ui.status.busy {
        return Task::none();
    }
    if !force
        && state
            .floor_ui
            .status
            .last_poll
            .is_some_and(|last| last.elapsed() < Duration::from_secs(1))
    {
        return Task::none();
    }
    let Some(workspace) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
    else {
        return Task::none();
    };
    let Some(directory) = workspace.settings().working_directory() else {
        return Task::none();
    };
    let id = workspace.id();
    let directory = PathBuf::from(directory.as_str());
    let previous = if force {
        None
    } else {
        state.floor_ui.status.signature.clone()
    };
    state.floor_ui.status.busy = true;
    state.floor_ui.status.last_poll = Some(Instant::now());
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || inspect(&directory, previous.as_deref()))
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result)
        },
        move |result| AppMessage::Floor(super::Message::Status(Message::Completed(id, result))),
    )
}

pub(super) fn update(state: &mut OpenPodium, message: Message) -> Task<AppMessage> {
    let Message::Completed(id, result) = message;
    state.floor_ui.status.busy = false;
    if state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace_id)
        != Some(id)
    {
        return Task::none();
    }
    match result {
        Ok(result) => {
            state.floor_ui.status.signature = Some(result.signature);
            state.floor_ui.status.main = Some(result.main);
            if let Some(report) = result.report {
                state.floor_ui.status.report = report;
                state.canvas_revision = state.canvas_revision.wrapping_add(1);
            }
        }
        Err(error) => state.notice = Some(format!("Git collision refresh failed: {error}")),
    }
    Task::none()
}

pub(super) fn severity_for_main(state: &UiState) -> Option<CollisionSeverity> {
    state
        .main
        .as_deref()
        .and_then(|path| state.report.severity_for(path))
}

pub(super) fn severity_for_path(state: &UiState, path: &Path) -> Option<CollisionSeverity> {
    state.report.severity_for(path)
}

pub(super) fn changes_for<'a>(state: &'a UiState, path: &Path) -> &'a [ChangedPath] {
    state
        .report
        .inventories
        .iter()
        .find(|inventory| same_path(&inventory.checkout, path))
        .map_or(&[], |inventory| inventory.paths.as_slice())
}

pub(super) fn changes_for_main(state: &UiState) -> &[ChangedPath] {
    state
        .main
        .as_deref()
        .map_or(&[], |path| changes_for(state, path))
}

pub(super) fn change_label(kind: &ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "added",
        ChangeKind::Modified => "modified",
        ChangeKind::Deleted => "deleted",
        ChangeKind::Renamed { .. } => "renamed",
        ChangeKind::Untracked => "untracked",
        ChangeKind::Unmerged => "unmerged",
    }
}

pub(super) fn node_severities(state: &OpenPodium) -> BTreeMap<NodeId, CollisionSeverity> {
    let Some(workspace) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
    else {
        return BTreeMap::new();
    };
    workspace
        .canvas_layout()
        .nodes()
        .iter()
        .filter_map(|node| {
            let path = workspace
                .floors()
                .node_floors
                .get(&node.id())
                .and_then(|floor| workspace.floors().entries.get(floor))
                .map(|floor| Path::new(floor.directory.as_str()))
                .or(state.floor_ui.status.main.as_deref())?;
            state
                .floor_ui
                .status
                .report
                .severity_for(path)
                .map(|severity| (node.id(), severity))
        })
        .collect()
}

pub(super) fn view(state: &OpenPodium) -> Element<'_, AppMessage> {
    let mut content = column![text("File collisions").size(18)].spacing(4);
    if state.floor_ui.status.busy {
        content = content.push(text("Checking changed paths…").size(12));
    }
    if state.floor_ui.status.report.collisions.is_empty() {
        content = content.push(text("No overlapping changed paths").size(12));
    } else {
        for collision in &state.floor_ui.status.report.collisions {
            content = content.push(
                text(format!(
                    "{} · {} · {} ↔ {}",
                    collision.severity,
                    collision.path,
                    checkout_name(&collision.left),
                    checkout_name(&collision.right)
                ))
                .size(12),
            );
        }
    }
    content.into()
}

fn inspect(directory: &Path, previous: Option<&str>) -> Result<ScanResult, String> {
    let repo = Repository::discover(directory)?;
    let checkouts = repo
        .checkouts()?
        .into_iter()
        .filter(|checkout| checkout.path.is_dir())
        .collect::<Vec<_>>();
    let signature = repo.change_signature(&checkouts)?;
    let report = if previous == Some(signature.as_str()) {
        None
    } else {
        Some(repo.collision_report(&checkouts)?)
    };
    Ok(ScanResult {
        signature,
        main: repo.root,
        report,
    })
}

fn checkout_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (dunce::canonicalize(left), dunce::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => dunce::simplified(left) == dunce::simplified(right),
    }
}
