//! Local workspace creation, settings, switching, and restoration.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::domain::{
    Content, DomainCommand, Name, NodeId, TimelineEvent, TimelineEventId, Timestamp,
    ValidationError, Workspace, WorkspaceDirectory, WorkspaceIcon, WorkspaceId, WorkspaceSettings,
};
use crate::persistence::{
    ImportPreview, Journal, PointV1, PortableError, PortableImport, decode_template,
    decode_workspace_archive, export_template, export_workspace_archive, import_template,
    import_workspace_archive, preview_template_import, preview_workspace_archive_import,
};

mod error;
mod floors;
pub use error::WorkspaceError;
pub use floors::{FloorJob, FloorOperation, FloorResult};

pub struct WorkspaceManager {
    journal: Journal,
    workspaces: BTreeMap<WorkspaceId, Workspace>,
    recent: Vec<WorkspaceId>,
    active: Option<WorkspaceId>,
}

impl WorkspaceManager {
    pub fn open(database_path: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        let journal = Journal::open(database_path)?;
        let recent = journal.recent_workspace_ids()?;
        let mut workspaces = BTreeMap::new();

        for workspace_id in &recent {
            let workspace = journal.recover(*workspace_id)?.ok_or(
                WorkspaceError::MissingPersistedWorkspace {
                    workspace_id: *workspace_id,
                },
            )?;
            workspaces.insert(*workspace_id, workspace);
        }

        let active = journal.active_workspace_id()?;
        if let Some(workspace_id) = active
            && !workspaces.contains_key(&workspace_id)
        {
            return Err(WorkspaceError::MissingPersistedWorkspace { workspace_id });
        }

        Ok(Self {
            journal,
            workspaces,
            recent,
            active,
        })
    }

    pub fn create_workspace(
        &mut self,
        directory: impl AsRef<Path>,
        occurred_at: Timestamp,
    ) -> Result<WorkspaceId, WorkspaceError> {
        let (canonical_path, working_directory) = validate_directory(directory.as_ref())?;
        let name = default_workspace_name(&canonical_path)?;
        let workspace_id = self.next_workspace_id()?;
        let settings = WorkspaceSettings::new(name.clone(), None, Some(working_directory), None);
        let mut workspace = Workspace::new(workspace_id, name);

        self.journal.execute(
            &mut workspace,
            DomainCommand::UpdateWorkspaceSettings(settings),
            occurred_at,
        )?;
        self.workspaces.insert(workspace_id, workspace);
        self.mark_recent(workspace_id);
        self.active = Some(workspace_id);
        Ok(workspace_id)
    }

    pub fn update_settings(
        &mut self,
        workspace_id: WorkspaceId,
        input: WorkspaceSettingsInput,
        occurred_at: Timestamp,
    ) -> Result<TimelineEvent, WorkspaceError> {
        let (_, working_directory) = validate_directory(&input.working_directory)?;
        let settings = WorkspaceSettings::new(
            Name::new(input.name)?,
            optional_icon(input.icon)?,
            Some(working_directory),
            optional_instructions(input.instructions)?,
        );
        self.execute(
            workspace_id,
            DomainCommand::UpdateWorkspaceSettings(settings),
            occurred_at,
        )
    }

    pub fn execute(
        &mut self,
        workspace_id: WorkspaceId,
        command: DomainCommand,
        occurred_at: Timestamp,
    ) -> Result<TimelineEvent, WorkspaceError> {
        let workspace = self
            .workspaces
            .get_mut(&workspace_id)
            .ok_or(WorkspaceError::UnknownWorkspace { workspace_id })?;
        self.journal
            .execute(workspace, command, occurred_at)
            .map_err(WorkspaceError::from)
    }

    pub fn export_template(
        &self,
        workspace_id: WorkspaceId,
        selection: &[NodeId],
    ) -> Result<String, WorkspaceError> {
        let workspace = self
            .workspace(workspace_id)
            .ok_or(WorkspaceError::UnknownWorkspace { workspace_id })?;
        export_template(workspace, selection).map_err(WorkspaceError::from)
    }

    pub fn preview_template_import(
        &self,
        workspace_id: WorkspaceId,
        payload: &str,
    ) -> Result<ImportPreview, WorkspaceError> {
        let workspace = self
            .workspace(workspace_id)
            .ok_or(WorkspaceError::UnknownWorkspace { workspace_id })?;
        let document = decode_template(payload).map_err(WorkspaceError::from)?;
        preview_template_import(&document, workspace).map_err(WorkspaceError::from)
    }

    pub fn import_template(
        &mut self,
        workspace_id: WorkspaceId,
        payload: &str,
        destination_floor: Option<u64>,
        destination_origin: PointV1,
        launcher_mappings: &std::collections::BTreeMap<String, crate::domain::CommandPresetId>,
        occurred_at: Timestamp,
    ) -> Result<Vec<TimelineEvent>, WorkspaceError> {
        let plan = {
            let workspace = self
                .workspace(workspace_id)
                .ok_or(WorkspaceError::UnknownWorkspace { workspace_id })?;
            validate_destination_floor(workspace, destination_floor)?;
            let document = decode_template(payload).map_err(WorkspaceError::from)?;
            import_template(&document, workspace, destination_origin, launcher_mappings)
                .map_err(WorkspaceError::from)?
        };
        validate_import_paths(
            self.workspace(workspace_id).expect("workspace was checked"),
            &plan.preview,
        )?;
        self.execute_import(workspace_id, plan, occurred_at)
    }

    pub fn export_workspace_archive(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<String, WorkspaceError> {
        let workspace = self
            .workspace(workspace_id)
            .ok_or(WorkspaceError::UnknownWorkspace { workspace_id })?;
        export_workspace_archive(workspace).map_err(WorkspaceError::from)
    }

    pub fn preview_workspace_archive_import(
        &self,
        workspace_id: WorkspaceId,
        payload: &str,
    ) -> Result<ImportPreview, WorkspaceError> {
        let workspace = self
            .workspace(workspace_id)
            .ok_or(WorkspaceError::UnknownWorkspace { workspace_id })?;
        let archive = decode_workspace_archive(payload).map_err(WorkspaceError::from)?;
        preview_workspace_archive_import(&archive, workspace).map_err(WorkspaceError::from)
    }

    pub fn import_workspace_archive(
        &mut self,
        workspace_id: WorkspaceId,
        payload: &str,
        destination_floor: Option<u64>,
        launcher_mappings: &std::collections::BTreeMap<String, crate::domain::CommandPresetId>,
        occurred_at: Timestamp,
    ) -> Result<Vec<TimelineEvent>, WorkspaceError> {
        let plan = {
            let workspace = self
                .workspace(workspace_id)
                .ok_or(WorkspaceError::UnknownWorkspace { workspace_id })?;
            validate_destination_floor(workspace, destination_floor)?;
            let archive = decode_workspace_archive(payload).map_err(WorkspaceError::from)?;
            import_workspace_archive(&archive, workspace, launcher_mappings)
                .map_err(WorkspaceError::from)?
        };
        validate_import_paths(
            self.workspace(workspace_id).expect("workspace was checked"),
            &plan.preview,
        )?;
        self.execute_import(workspace_id, plan, occurred_at)
    }

    fn execute_import(
        &mut self,
        workspace_id: WorkspaceId,
        plan: PortableImport,
        occurred_at: Timestamp,
    ) -> Result<Vec<TimelineEvent>, WorkspaceError> {
        let workspace = self
            .workspaces
            .get_mut(&workspace_id)
            .ok_or(WorkspaceError::UnknownWorkspace { workspace_id })?;
        self.journal
            .execute_batch(workspace, plan.commands, occurred_at)
            .map_err(WorkspaceError::from)
    }

    pub fn switch(
        &mut self,
        workspace_id: WorkspaceId,
        opened_at: Timestamp,
    ) -> Result<(), WorkspaceError> {
        if !self.workspaces.contains_key(&workspace_id) {
            return Err(WorkspaceError::UnknownWorkspace { workspace_id });
        }

        self.journal
            .activate_workspace(workspace_id, opened_at)
            .map_err(WorkspaceError::from)?;
        self.active = Some(workspace_id);
        self.mark_recent(workspace_id);
        Ok(())
    }

    pub fn active_workspace_id(&self) -> Option<WorkspaceId> {
        self.active
    }

    pub fn active_workspace(&self) -> Option<&Workspace> {
        self.active.and_then(|id| self.workspaces.get(&id))
    }

    pub fn workspace(&self, workspace_id: WorkspaceId) -> Option<&Workspace> {
        self.workspaces.get(&workspace_id)
    }

    pub fn timeline(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<TimelineEvent>, WorkspaceError> {
        self.timeline_after(workspace_id, None)
    }

    pub fn timeline_after(
        &self,
        workspace_id: WorkspaceId,
        after: Option<TimelineEventId>,
    ) -> Result<Vec<TimelineEvent>, WorkspaceError> {
        if !self.workspaces.contains_key(&workspace_id) {
            return Err(WorkspaceError::UnknownWorkspace { workspace_id });
        }
        self.journal
            .timeline_after(workspace_id, after)
            .map_err(WorkspaceError::from)
    }

    pub fn recent_workspaces(&self) -> impl Iterator<Item = &Workspace> {
        self.recent
            .iter()
            .filter_map(|workspace_id| self.workspaces.get(workspace_id))
    }

    pub fn shortcuts(&self) -> Result<Vec<(String, Option<String>)>, WorkspaceError> {
        self.journal.shortcuts().map_err(WorkspaceError::from)
    }

    pub fn store_shortcut(
        &mut self,
        command_id: &str,
        shortcut: Option<&str>,
    ) -> Result<(), WorkspaceError> {
        self.journal
            .store_shortcut(command_id, shortcut)
            .map_err(WorkspaceError::from)
    }

    fn next_workspace_id(&self) -> Result<WorkspaceId, WorkspaceError> {
        let next = self
            .workspaces
            .last_key_value()
            .map_or(0, |(workspace_id, _)| workspace_id.get())
            .checked_add(1)
            .ok_or(WorkspaceError::WorkspaceIdExhausted)?;
        Ok(WorkspaceId::new(next))
    }

    fn mark_recent(&mut self, workspace_id: WorkspaceId) {
        self.recent.retain(|candidate| *candidate != workspace_id);
        self.recent.insert(0, workspace_id);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSettingsInput {
    pub name: String,
    pub icon: Option<String>,
    pub working_directory: PathBuf,
    pub instructions: Option<String>,
}

fn validate_directory(path: &Path) -> Result<(PathBuf, WorkspaceDirectory), WorkspaceError> {
    let metadata = fs::metadata(path).map_err(|source| WorkspaceError::DirectoryAccess {
        operation: "inspect",
        path: path.to_owned(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(WorkspaceError::NotDirectory {
            path: path.to_owned(),
        });
    }
    fs::read_dir(path).map_err(|source| WorkspaceError::DirectoryAccess {
        operation: "read",
        path: path.to_owned(),
        source,
    })?;
    let canonical_path =
        fs::canonicalize(path).map_err(|source| WorkspaceError::DirectoryAccess {
            operation: "canonicalize",
            path: path.to_owned(),
            source,
        })?;
    let encoded = canonical_path
        .to_str()
        .ok_or_else(|| WorkspaceError::NonUnicodeDirectory {
            path: canonical_path.clone(),
        })?;
    let working_directory = WorkspaceDirectory::new(encoded)?;
    Ok((canonical_path, working_directory))
}

fn validate_destination_floor(
    workspace: &Workspace,
    destination_floor: Option<u64>,
) -> Result<(), WorkspaceError> {
    if destination_floor != workspace.floors().active {
        return Err(WorkspaceError::InvalidImportDestination {
            floor: destination_floor,
        });
    }
    if let Some(floor) = destination_floor
        && !workspace.floors().entries.contains_key(&floor)
    {
        return Err(WorkspaceError::InvalidImportDestination { floor: Some(floor) });
    }
    Ok(())
}

fn validate_import_paths(
    workspace: &Workspace,
    preview: &ImportPreview,
) -> Result<(), WorkspaceError> {
    if preview.referenced_paths.is_empty() {
        return Ok(());
    }
    let checkout = workspace.active_directory().ok_or_else(|| {
        WorkspaceError::Portable(PortableError::InvalidDocument(
            "the selected destination floor has no available checkout for path validation"
                .to_owned(),
        ))
    })?;
    for value in &preview.referenced_paths {
        let path = crate::domain::ProjectPath::new(value.clone()).map_err(|error| {
            WorkspaceError::Portable(PortableError::InvalidDocument(format!(
                "referenced path {value:?} is invalid: {error}"
            )))
        })?;
        crate::context::resolve_project_path(Path::new(checkout.as_str()), &path).map_err(
            |error| {
                WorkspaceError::Portable(PortableError::InvalidDocument(format!(
                    "referenced path {value:?} is unsafe for the destination checkout: {error}"
                )))
            },
        )?;
    }
    Ok(())
}

fn default_workspace_name(path: &Path) -> Result<Name, ValidationError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Workspace");
    Name::new(name)
}

fn optional_icon(value: Option<String>) -> Result<Option<WorkspaceIcon>, ValidationError> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(WorkspaceIcon::new)
        .transpose()
}

fn optional_instructions(value: Option<String>) -> Result<Option<Content>, ValidationError> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(Content::new)
        .transpose()
}

#[cfg(test)]
mod tests;
