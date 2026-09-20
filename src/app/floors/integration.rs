use std::path::{Path, PathBuf};

use iced::widget::{button, column, row, text};
use iced::{Element, Task};
use openpodium::domain::{FloorLifecycle, WorkspaceId};
use openpodium::git::{
    IntegrationAction, IntegrationError, IntegrationOutcome, IntegrationPreview, Repository,
};

use super::super::{Message as AppMessage, OpenPodium, WorkspaceManager};

#[derive(Default)]
pub(super) struct UiState {
    pub(super) busy: bool,
    preview: Option<(WorkspaceId, u64, IntegrationPreview)>,
}

#[derive(Clone)]
pub(crate) enum Message {
    Preview(u64),
    Previewed(WorkspaceId, u64, Box<Result<IntegrationPreview, String>>),
    Choose(IntegrationAction),
    Completed(Result<IntegrationOutcome, IntegrationError>),
}

pub(super) fn update(state: &mut OpenPodium, message: Message) -> Task<AppMessage> {
    match message {
        Message::Preview(floor_id) => preview(state, floor_id),
        Message::Previewed(workspace_id, floor_id, result) => {
            state.floor_ui.integration.busy = false;
            if state
                .workspaces
                .as_ref()
                .and_then(WorkspaceManager::active_workspace_id)
                != Some(workspace_id)
            {
                return Task::none();
            }
            match *result {
                Ok(preview) => {
                    state.floor_ui.integration.preview = Some((workspace_id, floor_id, preview));
                    state.notice = None;
                }
                Err(error) => state.notice = Some(format!("Integration preview failed: {error}")),
            }
            Task::none()
        }
        Message::Choose(action) => integrate(state, action),
        Message::Completed(result) => {
            state.floor_ui.integration.busy = false;
            state.floor_ui.integration.preview = None;
            state.notice = Some(match result {
                Ok(outcome) => outcome.message,
                Err(error) => error.to_string(),
            });
            super::status::scan(state, true)
        }
    }
}

pub(super) fn view(state: &OpenPodium) -> Element<'_, AppMessage> {
    let mut content = column![text("Integration preview").size(18)].spacing(4);
    if state.floor_ui.integration.busy {
        return content.push(text("Git operation in progress…")).into();
    }
    let Some((_, _, preview)) = &state.floor_ui.integration.preview else {
        return content
            .push(text("Choose a floor to preview its integration into main.").size(12))
            .into();
    };
    content = content
        .push(text(format!(
            "{} → {}",
            checkout_name(&preview.source.path),
            checkout_name(&preview.target.path)
        )))
        .push(
            text(format!(
                "{} source-only commit{}",
                preview.commits.len(),
                if preview.commits.len() == 1 { "" } else { "s" }
            ))
            .size(12),
        );
    if preview.source_dirty {
        content = content.push(
            text("Source has uncommitted changes; merge and cherry-pick include commits only")
                .size(12),
        );
    }
    if preview.target_dirty {
        content = content.push(text("Target has uncommitted changes").size(12));
    }
    for commit in &preview.commits {
        content = content.push(
            text(format!(
                "{} · {}",
                &commit.id[..commit.id.len().min(8)],
                commit.subject
            ))
            .size(12),
        );
    }
    if !preview.diff_summary.trim().is_empty() {
        content = content.push(text(preview.diff_summary.trim()).size(12));
    }
    if preview.likely_conflicts.is_empty() {
        content = content.push(text("No committed merge conflicts predicted").size(12));
    } else {
        for path in &preview.likely_conflicts {
            content = content.push(text(format!("Likely conflict · {path}")).size(12));
        }
    }

    let mut actions = row![].spacing(8);
    for (label, action) in [
        ("Merge", IntegrationAction::Merge),
        ("Rebase source", IntegrationAction::Rebase),
        ("Cherry-pick", IntegrationAction::CherryPick),
        ("Leave alone", IntegrationAction::LeaveAlone),
    ] {
        let mut action_button = button(label);
        if preview.blocker(action).is_none() {
            action_button = action_button.on_press(AppMessage::Floor(super::Message::Integration(
                Message::Choose(action),
            )));
        }
        actions = actions.push(action_button);
    }
    content = content.push(actions);
    for action in [
        IntegrationAction::Merge,
        IntegrationAction::Rebase,
        IntegrationAction::CherryPick,
    ] {
        if let Some(blocker) = preview.blocker(action) {
            content = content.push(text(blocker).size(12));
        }
    }
    content.into()
}

fn preview(state: &mut OpenPodium, floor_id: u64) -> Task<AppMessage> {
    if state.floor_ui.integration.busy {
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
    let Some(floor) = workspace.floors().entries.get(&floor_id) else {
        state.notice = Some("Unknown floor".to_owned());
        return Task::none();
    };
    if floor.lifecycle != FloorLifecycle::Available {
        state.notice = Some("The source checkout is unavailable".to_owned());
        return Task::none();
    }
    let workspace_id = workspace.id();
    let directory = PathBuf::from(directory.as_str());
    let source = PathBuf::from(floor.directory.as_str());
    state.floor_ui.integration.busy = true;
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || build_preview(&directory, &source))
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result)
        },
        move |result| {
            AppMessage::Floor(super::Message::Integration(Message::Previewed(
                workspace_id,
                floor_id,
                Box::new(result),
            )))
        },
    )
}

fn integrate(state: &mut OpenPodium, action: IntegrationAction) -> Task<AppMessage> {
    if state.floor_ui.integration.busy {
        return Task::none();
    }
    let Some((_, _, preview)) = state.floor_ui.integration.preview.clone() else {
        state.notice = Some("Preview the integration before choosing an action".to_owned());
        return Task::none();
    };
    state.floor_ui.integration.busy = true;
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                Repository::discover(&preview.target.path)
                    .map_err(|message| IntegrationError {
                        message,
                        checkout: preview.target.path.clone(),
                        guidance: "Refresh the preview before retrying".to_owned(),
                    })?
                    .integrate(&preview, action)
            })
            .await
            .unwrap_or_else(|error| {
                Err(IntegrationError {
                    message: error.to_string(),
                    checkout: PathBuf::new(),
                    guidance: "Inspect both checkouts with `git status`".to_owned(),
                })
            })
        },
        |result| AppMessage::Floor(super::Message::Integration(Message::Completed(result))),
    )
}

fn build_preview(directory: &Path, source: &Path) -> Result<IntegrationPreview, String> {
    let repo = Repository::discover(directory)?;
    let checkouts = repo.checkouts()?;
    let target = checkouts
        .first()
        .ok_or("The repository has no main checkout")?;
    let source = checkouts
        .iter()
        .find(|checkout| same_path(&checkout.path, source))
        .ok_or("The source checkout is no longer registered")?;
    repo.preview_integration(source, target)
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
