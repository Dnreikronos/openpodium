use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use iced::widget::{column, text};
use iced::{Element, Task};
use openpodium::domain::{NodeId, WorkspaceId};
use openpodium::git::{ChangeKind, ChangedPath, CollisionSeverity, Repository};
use openpodium::supervisor::{CollisionObservation, SignalClass};

use crate::notifications::NotificationRequest;

use super::super::{
    Message as AppMessage, OpenPodium, WorkspaceManager, now, refresh_supervisor_snapshot,
};

mod index;
use index::IndexedReport;

#[derive(Default)]
pub(super) struct UiState {
    workspaces: BTreeMap<WorkspaceId, ScanState>,
}

#[derive(Default)]
struct ScanState {
    directory: Option<PathBuf>,
    busy: bool,
    last_poll: Option<Instant>,
    signature: Option<String>,
    main: Option<PathBuf>,
    report: IndexedReport,
}

#[derive(Clone)]
pub(crate) enum Message {
    Completed(
        openpodium::domain::WorkspaceId,
        PathBuf,
        Result<ScanResult, String>,
    ),
}

#[derive(Clone)]
pub(crate) struct ScanResult {
    signature: String,
    main: PathBuf,
    report: Option<IndexedReport>,
}

pub(super) fn scan(state: &mut OpenPodium, force: bool) -> Task<AppMessage> {
    let active_id = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace_id);
    let candidates = scan_candidates(state);
    if align_with_candidates(
        &mut state.floor_ui.status,
        &mut state.supervisor_collisions,
        &candidates,
    ) {
        refresh_supervisor_snapshot(state);
    }

    let mut tasks = Vec::new();
    for (id, directory) in candidates {
        let scan = state.floor_ui.status.workspaces.entry(id).or_default();
        let force_this = force && active_id == Some(id);
        if scan.busy
            || (!force_this
                && scan
                    .last_poll
                    .is_some_and(|last| last.elapsed() < Duration::from_secs(1)))
        {
            continue;
        }
        let previous = if force_this {
            None
        } else {
            scan.signature.clone()
        };
        scan.busy = true;
        scan.last_poll = Some(Instant::now());
        let inspect_directory = directory.clone();
        let mut aliases = vec![directory.clone()];
        if let Some(workspace) = state
            .workspaces
            .as_ref()
            .and_then(|manager| manager.workspace(id))
        {
            aliases.extend(
                workspace
                    .floors()
                    .entries
                    .values()
                    .map(|floor| PathBuf::from(floor.directory.as_str())),
            );
        }
        tasks.push(Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    inspect(&inspect_directory, previous.as_deref(), &aliases)
                })
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result)
            },
            move |result| {
                AppMessage::Floor(super::Message::Status(Message::Completed(
                    id,
                    directory.clone(),
                    result,
                )))
            },
        ));
    }
    Task::batch(tasks)
}

pub(super) fn update(state: &mut OpenPodium, message: Message) -> Task<AppMessage> {
    let Message::Completed(id, directory, result) = message;
    let candidates = scan_candidates(state);
    if align_with_candidates(
        &mut state.floor_ui.status,
        &mut state.supervisor_collisions,
        &candidates,
    ) {
        refresh_supervisor_snapshot(state);
    }
    if !scan_is_current(&state.floor_ui.status, &candidates, id, &directory) {
        return Task::none();
    }
    let scan = state
        .floor_ui
        .status
        .workspaces
        .get_mut(&id)
        .expect("current scan state must exist");
    scan.busy = false;
    match result {
        Ok(result) => {
            let scan = state.floor_ui.status.workspaces.entry(id).or_default();
            scan.signature = Some(result.signature);
            scan.main = Some(result.main);
            if let Some(report) = result.report {
                let observations = report
                    .report
                    .collisions
                    .iter()
                    .map(|collision| CollisionObservation {
                        workspace_id: id,
                        path: collision.path.to_string(),
                        left_checkout: collision.left.display().to_string(),
                        right_checkout: collision.right.display().to_string(),
                    })
                    .collect::<Vec<_>>();
                let new_collision = observations.iter().find(|observation| {
                    !state
                        .supervisor_collisions
                        .get(&id)
                        .is_some_and(|previous| previous.contains(observation))
                });
                let notification = new_collision.and_then(|collision| {
                    state
                        .notification_limiter
                        .should_send(
                            id,
                            SignalClass::Collision,
                            now(),
                            state.supervisor_ui.notification_settings(),
                        )
                        .then(|| NotificationRequest {
                            target: None,
                            title: "File collision detected".to_owned(),
                            body: format!(
                                "{} overlaps between {} and {}",
                                collision.path, collision.left_checkout, collision.right_checkout
                            ),
                        })
                });
                let collision_changed = state.supervisor_collisions.get(&id) != Some(&observations);
                state.supervisor_collisions.insert(id, observations);
                state
                    .floor_ui
                    .status
                    .workspaces
                    .entry(id)
                    .or_default()
                    .report = report;
                if state
                    .workspaces
                    .as_ref()
                    .and_then(WorkspaceManager::active_workspace_id)
                    == Some(id)
                {
                    state.canvas_revision = state.canvas_revision.wrapping_add(1);
                }
                if collision_changed {
                    refresh_supervisor_snapshot(state);
                }
                if let Some(notification) = notification {
                    return Task::perform(
                        crate::notifications::show(notification),
                        AppMessage::NotificationActivated,
                    );
                }
            }
        }
        Err(error) => state.notice = Some(format!("Git collision refresh failed: {error}")),
    }
    Task::none()
}

fn scan_candidates(state: &OpenPodium) -> BTreeMap<WorkspaceId, PathBuf> {
    state
        .workspaces
        .as_ref()
        .into_iter()
        .flat_map(|workspaces| workspaces.recent_workspaces())
        .filter_map(|workspace| {
            Some((
                workspace.id(),
                PathBuf::from(workspace.settings().working_directory()?.as_str()),
            ))
        })
        .collect()
}

fn align_with_candidates(
    status: &mut UiState,
    collisions: &mut BTreeMap<WorkspaceId, Vec<CollisionObservation>>,
    candidates: &BTreeMap<WorkspaceId, PathBuf>,
) -> bool {
    status
        .workspaces
        .retain(|workspace_id, _| candidates.contains_key(workspace_id));
    let collision_count = collisions.len();
    collisions.retain(|workspace_id, _| candidates.contains_key(workspace_id));
    let mut collisions_changed = collision_count != collisions.len();

    for (workspace_id, directory) in candidates {
        let scan = status.workspaces.entry(*workspace_id).or_default();
        if scan.directory.as_ref() != Some(directory) {
            *scan = ScanState {
                directory: Some(directory.clone()),
                ..ScanState::default()
            };
            collisions_changed |= collisions.remove(workspace_id).is_some();
        }
    }

    collisions_changed
}

fn scan_is_current(
    status: &UiState,
    candidates: &BTreeMap<WorkspaceId, PathBuf>,
    workspace_id: WorkspaceId,
    directory: &Path,
) -> bool {
    candidates
        .get(&workspace_id)
        .is_some_and(|candidate| candidate == directory)
        && status
            .workspaces
            .get(&workspace_id)
            .and_then(|scan| scan.directory.as_deref())
            .is_some_and(|current| current == directory)
}

pub(super) fn severity_for_main(
    state: &UiState,
    workspace_id: WorkspaceId,
) -> Option<CollisionSeverity> {
    let scan = state.workspaces.get(&workspace_id)?;
    scan.main
        .as_deref()
        .and_then(|path| scan.report.severity_for(path))
}

pub(super) fn severity_for_path(
    state: &UiState,
    workspace_id: WorkspaceId,
    path: &Path,
) -> Option<CollisionSeverity> {
    state
        .workspaces
        .get(&workspace_id)?
        .report
        .severity_for(path)
}

pub(super) fn changes_for<'a>(
    state: &'a UiState,
    workspace_id: WorkspaceId,
    path: &Path,
) -> &'a [ChangedPath] {
    let Some(scan) = state.workspaces.get(&workspace_id) else {
        return &[];
    };
    scan.report.changes_for(path)
}

pub(super) fn changes_for_main(state: &UiState, workspace_id: WorkspaceId) -> &[ChangedPath] {
    state
        .workspaces
        .get(&workspace_id)
        .and_then(|scan| scan.main.as_deref())
        .map_or(&[], |path| changes_for(state, workspace_id, path))
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
    let Some(scan) = state.floor_ui.status.workspaces.get(&workspace.id()) else {
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
                .or(scan.main.as_deref())?;
            scan.report
                .severity_for(path)
                .map(|severity| (node.id(), severity))
        })
        .collect()
}

pub(super) fn view(state: &OpenPodium) -> Element<'_, AppMessage> {
    let mut content = column![text("File collisions").size(18)].spacing(4);
    let scan = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace_id)
        .and_then(|workspace_id| state.floor_ui.status.workspaces.get(&workspace_id));
    if scan.is_some_and(|scan| scan.busy) {
        content = content.push(text("Checking changed paths…").size(12));
    }
    if scan.is_none_or(|scan| scan.report.report.collisions.is_empty()) {
        content = content.push(text("No overlapping changed paths").size(12));
    } else if let Some(scan) = scan {
        for collision in &scan.report.report.collisions {
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

fn inspect(
    directory: &Path,
    previous: Option<&str>,
    aliases: &[PathBuf],
) -> Result<ScanResult, String> {
    let repo = Repository::discover(directory)?;
    let checkouts = repo
        .checkouts()?
        .into_iter()
        .filter(|checkout| checkout.path.is_dir())
        .collect::<Vec<_>>();
    // A new floor alias needs indexing even if Git's contents have not changed.
    let signature = format!("{}:{aliases:?}", repo.change_signature(&checkouts)?);
    let report = if previous == Some(signature.as_str()) {
        None
    } else {
        Some(IndexedReport::new(
            repo.collision_report(&checkouts)?,
            aliases,
        ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_alignment_prunes_stale_collision_state() {
        let retained = WorkspaceId::new(1);
        let removed = WorkspaceId::new(2);
        let directory = PathBuf::from("/project");
        let mut status = UiState::default();
        status.workspaces.insert(
            retained,
            ScanState {
                directory: Some(directory.clone()),
                ..ScanState::default()
            },
        );
        status.workspaces.insert(removed, ScanState::default());
        let mut collisions = BTreeMap::from([(retained, Vec::new()), (removed, Vec::new())]);
        let candidates = BTreeMap::from([(retained, directory)]);

        assert!(align_with_candidates(
            &mut status,
            &mut collisions,
            &candidates
        ));
        assert!(status.workspaces.contains_key(&retained));
        assert!(!status.workspaces.contains_key(&removed));
        assert!(collisions.contains_key(&retained));
        assert!(!collisions.contains_key(&removed));
    }

    #[test]
    fn directory_change_resets_scan_and_rejects_old_completion() {
        let workspace_id = WorkspaceId::new(1);
        let old_directory = PathBuf::from("/old");
        let new_directory = PathBuf::from("/new");
        let mut status = UiState::default();
        status.workspaces.insert(
            workspace_id,
            ScanState {
                directory: Some(old_directory.clone()),
                busy: true,
                signature: Some("old signature".to_owned()),
                ..ScanState::default()
            },
        );
        let mut collisions = BTreeMap::from([(workspace_id, Vec::new())]);
        let candidates = BTreeMap::from([(workspace_id, new_directory.clone())]);

        assert!(align_with_candidates(
            &mut status,
            &mut collisions,
            &candidates
        ));
        let scan = status.workspaces.get(&workspace_id).unwrap();
        assert_eq!(scan.directory.as_ref(), Some(&new_directory));
        assert!(!scan.busy);
        assert!(scan.signature.is_none());
        assert!(collisions.is_empty());
        assert!(!scan_is_current(
            &status,
            &candidates,
            workspace_id,
            &old_directory
        ));
        assert!(scan_is_current(
            &status,
            &candidates,
            workspace_id,
            &new_directory
        ));
    }
}
