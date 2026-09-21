use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use iced::widget::{column, row, text, text_editor};

use super::ui::button;
use super::ui::section_label;
use iced::{Element, Fill, Task};
use openpodium::context::{
    FilePreview, NoteBuffer, list_directory, preview_file, render_diff, resolve_project_path,
};
use openpodium::domain::{
    Arrow, CanvasColor, CanvasNodeContent, CanvasPoint, CanvasSize, CanvasText, DiffComparison,
    DomainCommand, Freehand, Name, Node, NodeId, NormalizedPoint, ProjectPath, Shape, ShapeKind,
    StrokeWidth, Workspace, WorkspaceId,
};

use super::{Message, OpenPodium, WorkspaceManager, canvas_coordinate, next_node_id, now};

pub(super) const EDITOR_ID: &str = "context-editor";

#[derive(Debug, Clone, Copy)]
pub(super) enum Kind {
    Note,
    FileTree,
    Artifact,
    Diff,
    Text,
    Rectangle,
    Ellipse,
    Arrow,
    Freehand,
}

pub(super) struct UiState {
    note_key: Option<(WorkspaceId, NodeId)>,
    text_key: Option<(WorkspaceId, NodeId)>,
    note: Option<NoteBuffer>,
    editor: text_editor::Content,
    refresh_ticks: u8,
    bodies: BTreeMap<NodeId, String>,
    refresh_busy: bool,
    search_match: Option<usize>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            note_key: None,
            text_key: None,
            note: None,
            editor: text_editor::Content::new(),
            refresh_ticks: 0,
            bodies: BTreeMap::new(),
            refresh_busy: false,
            search_match: None,
        }
    }
}

pub(super) fn selection_changed(state: &mut OpenPodium) {
    let selected = (state.canvas_selection.len() == 1).then(|| state.canvas_selection[0]);
    let target = selected.and_then(|node_id| {
        let workspace = state.workspaces.as_ref()?.active_workspace()?;
        let node = workspace.node(node_id)?;
        match node.content() {
            CanvasNodeContent::Note { path, .. } => Some(SelectedEditor::Note {
                key: (workspace.id(), node_id),
                checkout: PathBuf::from(workspace.node_directory(node_id)?.as_str()),
                path: path.clone(),
            }),
            CanvasNodeContent::Text { markdown } => Some(SelectedEditor::Text {
                key: (workspace.id(), node_id),
                text: markdown.as_str().to_owned(),
            }),
            _ => None,
        }
    });
    let Some(target) = target else {
        state.context_ui.note_key = None;
        state.context_ui.text_key = None;
        state.context_ui.note = None;
        state.context_ui.editor = text_editor::Content::new();
        state.context_ui.search_match = None;
        return;
    };
    match target {
        SelectedEditor::Note {
            key,
            checkout,
            path,
        } => {
            if state.context_ui.note_key == Some(key) {
                return;
            }
            match NoteBuffer::load(&checkout, path) {
                Ok(note) => {
                    state.context_ui.editor = text_editor::Content::with_text(note.text());
                    state.context_ui.note_key = Some(key);
                    state.context_ui.text_key = None;
                    state.context_ui.note = Some(note);
                }
                Err(error) => state.notice = Some(error.to_string()),
            }
        }
        SelectedEditor::Text { key, text } => {
            if state.context_ui.text_key == Some(key) {
                return;
            }
            state.context_ui.editor = text_editor::Content::with_text(&text);
            state.context_ui.note_key = None;
            state.context_ui.text_key = Some(key);
            state.context_ui.note = None;
        }
    }
}

enum SelectedEditor {
    Note {
        key: (WorkspaceId, NodeId),
        checkout: PathBuf,
        path: ProjectPath,
    },
    Text {
        key: (WorkspaceId, NodeId),
        text: String,
    },
}

#[derive(Clone)]
pub(super) struct ScanResult {
    workspace_id: WorkspaceId,
    note_key: Option<(WorkspaceId, NodeId)>,
    original_note: Option<NoteBuffer>,
    refreshed_note: Option<NoteBuffer>,
    bodies: BTreeMap<NodeId, String>,
}

pub(super) fn tick(state: &mut OpenPodium) -> Task<Message> {
    state.context_ui.refresh_ticks = state.context_ui.refresh_ticks.wrapping_add(1);
    if state.context_ui.refresh_busy || !state.context_ui.refresh_ticks.is_multiple_of(5) {
        return Task::none();
    }
    let Some((workspace_id, checkout, layout)) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
        .and_then(|workspace| {
            Some((
                workspace.id(),
                PathBuf::from(workspace.active_directory()?.as_str()),
                workspace.canvas_layout(),
            ))
        })
    else {
        return Task::none();
    };
    let changes = super::floors::changed_paths(state);
    let note_key = state.context_ui.note_key;
    let note = state.context_ui.note.clone();
    state.context_ui.refresh_busy = true;
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                scan(workspace_id, checkout, layout, changes, note_key, note)
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result)
        },
        Message::ContextScanCompleted,
    )
}

pub(super) fn scan_completed(state: &mut OpenPodium, result: Result<ScanResult, String>) {
    state.context_ui.refresh_busy = false;
    let mut result = match result {
        Ok(result) => result,
        Err(error) => {
            state.notice = Some(error);
            return;
        }
    };
    if state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace_id)
        != Some(result.workspace_id)
    {
        return;
    }
    if state.context_ui.note_key == result.note_key
        && state.context_ui.note == result.original_note
        && let Some(note) = result.refreshed_note.take()
    {
        let text_changed = state
            .context_ui
            .note
            .as_ref()
            .is_some_and(|current| current.text() != note.text());
        if text_changed {
            state.context_ui.editor = text_editor::Content::with_text(note.text());
        }
        state.context_ui.note = Some(note);
    } else if let Some((_, node_id)) = state.context_ui.note_key
        && let Some(note) = state.context_ui.note.as_ref()
    {
        result.bodies.insert(node_id, note.text().to_owned());
    }
    if state.context_ui.bodies != result.bodies {
        state.context_ui.bodies = result.bodies;
        state.canvas_revision = state.canvas_revision.wrapping_add(1);
    }
}

pub(super) fn edit_note(state: &mut OpenPodium, action: text_editor::Action) {
    state.context_ui.search_match = None;
    state.context_ui.editor.perform(action);
    if let Some(note) = state.context_ui.note.as_mut() {
        note.edit(state.context_ui.editor.text());
        if let Some((_, node_id)) = state.context_ui.note_key {
            state
                .context_ui
                .bodies
                .insert(node_id, note.text().to_owned());
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
        }
    }
}

pub(super) fn save_canvas_text(state: &mut OpenPodium) {
    let Some((workspace_id, node_id)) = state.context_ui.text_key else {
        return;
    };
    let text = match CanvasText::new(state.context_ui.editor.text()) {
        Ok(text) => text,
        Err(error) => {
            state.notice = Some(error.to_string());
            return;
        }
    };
    let Some(before) = state
        .workspaces
        .as_ref()
        .and_then(|manager| manager.workspace(workspace_id))
        .map(Workspace::canvas_layout)
    else {
        state.notice = Some("The text node workspace is unavailable".to_owned());
        return;
    };
    let nodes = before
        .nodes()
        .iter()
        .map(|node| {
            if node.id() == node_id {
                Node::with_content_and_z_index(
                    node.id(),
                    CanvasNodeContent::Text {
                        markdown: text.clone(),
                    },
                    node.position(),
                    node.size(),
                    node.z_index(),
                )
            } else {
                node.clone()
            }
        })
        .collect();
    let after = openpodium::domain::CanvasLayout::new(
        nodes,
        before.groups().to_vec(),
        before.connections().to_vec(),
    );
    if before == after {
        state.notice = Some("Text is unchanged".to_owned());
        return;
    }
    if super::persist_canvas(state, before.clone(), after).is_ok() {
        state.canvas_history.record(before);
        state.notice = Some("Text node saved".to_owned());
    }
}

pub(super) fn save_note(state: &mut OpenPodium, overwrite: bool) {
    let Some((workspace_id, node_id)) = state.context_ui.note_key else {
        return;
    };
    let Some(checkout) = state
        .workspaces
        .as_ref()
        .and_then(|manager| manager.workspace(workspace_id))
        .and_then(|workspace| workspace.node_directory(node_id))
        .map(|directory| PathBuf::from(directory.as_str()))
    else {
        state.notice = Some("The note checkout is unavailable".to_owned());
        return;
    };
    let Some(note) = state.context_ui.note.as_mut() else {
        return;
    };
    state.notice = Some(match note.save(&checkout, overwrite) {
        Ok(()) => "Note saved".to_owned(),
        Err(error) => error.to_string(),
    });
}

pub(super) fn reload_note(state: &mut OpenPodium) {
    let Some((workspace_id, node_id)) = state.context_ui.note_key else {
        return;
    };
    let Some(checkout) = state
        .workspaces
        .as_ref()
        .and_then(|manager| manager.workspace(workspace_id))
        .and_then(|workspace| workspace.node_directory(node_id))
        .map(|directory| PathBuf::from(directory.as_str()))
    else {
        state.notice = Some("The note checkout is unavailable".to_owned());
        return;
    };
    let Some(note) = state.context_ui.note.as_mut() else {
        return;
    };
    match note.reload_discarding_edits(&checkout) {
        Ok(()) => {
            state.context_ui.editor = text_editor::Content::with_text(note.text());
            state.notice = Some("Note reloaded".to_owned());
        }
        Err(error) => state.notice = Some(error.to_string()),
    }
}

pub(super) fn note_panel(state: &UiState) -> Option<Element<'_, Message>> {
    let Some(note) = state.note.as_ref() else {
        return state.text_key.map(|_| {
            column![
                section_label("Editing text node"),
                state
                    .search_match
                    .map(|offset| text(format!("Search match near character {offset}")).size(12)),
                text_editor(&state.editor)
                    .id(EDITOR_ID)
                    .placeholder("Write Markdown…")
                    .on_action(Message::EditNote)
                    .height(180),
                button("Save text node").on_press(Message::SaveCanvasText),
            ]
            .spacing(8)
            .width(Fill)
            .into()
        });
    };
    let mut panel = column![
        text(format!("Editing {}", note.path())).size(18),
        state
            .search_match
            .map(|offset| text(format!("Search match near character {offset}")).size(12)),
        text_editor(&state.editor)
            .id(EDITOR_ID)
            .placeholder("Write Markdown…")
            .on_action(Message::EditNote)
            .height(180),
        row![
            button("Save note").on_press(Message::SaveNote(false)),
            text(if note.is_dirty() { "Unsaved" } else { "Saved" }),
        ]
        .spacing(8),
    ]
    .spacing(8)
    .width(Fill);
    if note.has_external_change() {
        panel = panel.push(text(
            "The file changed outside OpenPodium; local edits were kept.",
        ));
        panel = panel.push(
            row![
                button("Reload and discard local edits").on_press(Message::ReloadNote),
                button("Overwrite external change").on_press(Message::SaveNote(true)),
            ]
            .spacing(8),
        );
    }
    Some(panel.into())
}

pub(super) fn reveal_offset(state: &mut OpenPodium, character_offset: usize) -> Task<Message> {
    let text = state.context_ui.editor.text();
    let byte_offset = text
        .char_indices()
        .nth(character_offset)
        .map_or(text.len(), |(offset, _)| offset);
    let prefix = &text[..byte_offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, line)| line)
        .chars()
        .count();
    state.context_ui.editor.move_to(text_editor::Cursor {
        position: text_editor::Position { line, column },
        selection: None,
    });
    state.context_ui.search_match = Some(character_offset);
    iced::widget::operation::focus(EDITOR_ID)
}

pub(super) fn connected_note_paths(
    workspace: &Workspace,
    agent_node: NodeId,
    checkout: &Path,
) -> Vec<PathBuf> {
    let layout = workspace.canvas_layout();
    let connected = layout.connections().iter().filter_map(|connection| {
        if connection.source() == agent_node {
            Some(connection.target())
        } else if connection.target() == agent_node {
            Some(connection.source())
        } else {
            None
        }
    });
    connected
        .filter_map(|node_id| workspace.node(node_id))
        .filter_map(|node| match node.content() {
            CanvasNodeContent::Note { path, .. } => resolve_project_path(checkout, path).ok(),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn bodies(state: &UiState) -> BTreeMap<NodeId, String> {
    state.bodies.clone()
}

fn scan(
    workspace_id: WorkspaceId,
    checkout: PathBuf,
    layout: openpodium::domain::CanvasLayout,
    changes: Vec<openpodium::git::ChangedPath>,
    note_key: Option<(WorkspaceId, NodeId)>,
    original_note: Option<NoteBuffer>,
) -> Result<ScanResult, String> {
    let mut refreshed_note = original_note.clone();
    if note_key.is_some_and(|(note_workspace, _)| note_workspace == workspace_id)
        && let Some(note) = refreshed_note.as_mut()
    {
        note.refresh(&checkout).map_err(|error| error.to_string())?;
    }
    let selected_note = note_key
        .filter(|(selected_workspace, _)| *selected_workspace == workspace_id)
        .and_then(|(_, node_id)| {
            refreshed_note
                .as_ref()
                .map(|note| (node_id, note.text().to_owned()))
        });
    let mut bodies = BTreeMap::new();
    for node in layout.nodes() {
        let body = match node.content() {
            CanvasNodeContent::Note { path, .. } => selected_note
                .as_ref()
                .filter(|(node_id, _)| *node_id == node.id())
                .map(|(_, text)| text.clone())
                .unwrap_or_else(|| preview_body(&checkout, path)),
            CanvasNodeContent::FileTree { root } => list_directory(&checkout, root, &changes)
                .map(|entries| {
                    entries
                        .into_iter()
                        .map(|entry| {
                            let marker = match entry.kind {
                                openpodium::context::FileTreeEntryKind::Directory => "▸",
                                openpodium::context::FileTreeEntryKind::File => "·",
                                openpodium::context::FileTreeEntryKind::Symlink => "↗",
                            };
                            entry.change.map_or_else(
                                || format!("{marker} {}", entry.name),
                                |change| format!("{marker} {} [{change:?}]", entry.name),
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_else(|error| error.to_string()),
            CanvasNodeContent::Artifact { path } => preview_body(&checkout, path),
            CanvasNodeContent::Diff { path, .. } => render_diff(&checkout, path)
                .map(|diff| diff.text)
                .unwrap_or_else(|error| error.to_string()),
            _ => continue,
        };
        bodies.insert(node.id(), body);
    }
    Ok(ScanResult {
        workspace_id,
        note_key,
        original_note,
        refreshed_note,
        bodies,
    })
}

fn preview_body(checkout: &Path, path: &ProjectPath) -> String {
    match preview_file(checkout, path, openpodium::context::MAX_TEXT_PREVIEW_BYTES) {
        Ok(FilePreview::Missing) => "File is unavailable".to_owned(),
        Ok(FilePreview::Text { content, .. }) => content,
        Ok(FilePreview::Binary { size, .. }) => format!("Binary file · {size} bytes"),
        Ok(FilePreview::TooLarge { size, max_bytes }) => {
            format!("File is too large to preview · {size} bytes · limit {max_bytes}")
        }
        Err(error) => error.to_string(),
    }
}

pub(super) fn add(state: &mut OpenPodium, kind: Kind) -> Task<Message> {
    let Some(workspace) = state
        .workspaces
        .as_ref()
        .and_then(WorkspaceManager::active_workspace)
    else {
        state.notice = Some("Create or select a workspace first".to_owned());
        return Task::none();
    };
    let Some(directory) = workspace.active_directory() else {
        state.notice = Some("Choose an available floor before adding project context".to_owned());
        return Task::none();
    };
    let checkout = PathBuf::from(directory.as_str());
    let Some(node_id) = next_node_id(workspace) else {
        state.notice = Some("No more canvas node identifiers are available".to_owned());
        return Task::none();
    };
    let before = workspace.canvas_layout();
    let content = match content(kind, node_id.get(), &state.context_path, &checkout) {
        Ok(content) => content,
        Err(error) => {
            state.notice = Some(error);
            return Task::none();
        }
    };
    let offset = (before.nodes().len() % 6) as f64 * 40.0;
    let z_index = before
        .nodes()
        .iter()
        .map(Node::z_index)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let node = Node::with_content_and_z_index(
        node_id,
        content,
        CanvasPoint::new(
            canvas_coordinate(state.camera.position().x - 180.0 + offset),
            canvas_coordinate(state.camera.position().y - 130.0 + offset),
        )
        .expect("clamped camera coordinates are finite"),
        CanvasSize::new(360.0, 260.0).expect("default node size is valid"),
        z_index,
    );
    let workspace_id = workspace.id();
    let result = state
        .workspaces
        .as_mut()
        .expect("active workspace came from the manager")
        .execute(workspace_id, DomainCommand::AddNode(node), now());
    match result {
        Ok(_) => {
            state.canvas_history.record(before);
            state.canvas_selection = vec![node_id];
            state.canvas_preview = None;
            state.canvas_revision = state.canvas_revision.wrapping_add(1);
            state.notice = Some(format!("{} node added", label(kind)));
        }
        Err(error) => state.notice = Some(error.to_string()),
    }
    Task::none()
}

fn content(
    kind: Kind,
    node_id: u64,
    path_input: &str,
    checkout: &Path,
) -> Result<CanvasNodeContent, String> {
    let black = CanvasColor::rgba(17, 24, 39, 255);
    let blue = CanvasColor::rgba(59, 130, 246, 255);
    let transparent_blue = CanvasColor::rgba(59, 130, 246, 48);
    let stroke = StrokeWidth::new(2.0).expect("default stroke width is valid");
    match kind {
        Kind::Note => {
            let path = ProjectPath::new(format!(".openpodium/notes/{node_id}.md"))
                .expect("generated note path is valid");
            NoteBuffer::new(path.clone())
                .save(checkout, false)
                .map_err(|error| error.to_string())?;
            Ok(CanvasNodeContent::Note {
                path,
                title: Name::new(format!("Note {node_id}")).expect("generated note title is valid"),
            })
        }
        Kind::FileTree => {
            let root = if path_input.trim().is_empty() {
                ProjectPath::new(".").expect("project root path is valid")
            } else {
                project_path(path_input)?
            };
            list_directory(checkout, &root, &[]).map_err(|error| error.to_string())?;
            Ok(CanvasNodeContent::FileTree { root })
        }
        Kind::Artifact => {
            let path = required_path(path_input)?;
            match preview_file(checkout, &path, openpodium::context::MAX_TEXT_PREVIEW_BYTES)
                .map_err(|error| error.to_string())?
            {
                FilePreview::Missing => return Err("The artifact file does not exist".to_owned()),
                FilePreview::Text { .. }
                | FilePreview::Binary { .. }
                | FilePreview::TooLarge { .. } => {}
            }
            Ok(CanvasNodeContent::Artifact { path })
        }
        Kind::Diff => {
            let path = required_path(path_input)?;
            render_diff(checkout, &path).map_err(|error| error.to_string())?;
            Ok(CanvasNodeContent::Diff {
                path,
                comparison: DiffComparison::WorkingTreeAgainstHead,
            })
        }
        Kind::Text => Ok(CanvasNodeContent::Text {
            markdown: CanvasText::new("Text").expect("default canvas text is valid"),
        }),
        Kind::Rectangle | Kind::Ellipse => Ok(CanvasNodeContent::Shape(Shape::new(
            if matches!(kind, Kind::Rectangle) {
                ShapeKind::Rectangle
            } else {
                ShapeKind::Ellipse
            },
            transparent_blue,
            blue,
            stroke,
        ))),
        Kind::Arrow => Ok(CanvasNodeContent::Arrow(Arrow::new(
            NormalizedPoint::new(0.1, 0.5).expect("default arrow point is valid"),
            NormalizedPoint::new(0.9, 0.5).expect("default arrow point is valid"),
            blue,
            stroke,
            None,
        ))),
        Kind::Freehand => Ok(CanvasNodeContent::Freehand(
            Freehand::new(
                vec![
                    NormalizedPoint::new(0.1, 0.7).expect("default drawing point is valid"),
                    NormalizedPoint::new(0.3, 0.3).expect("default drawing point is valid"),
                    NormalizedPoint::new(0.6, 0.7).expect("default drawing point is valid"),
                    NormalizedPoint::new(0.9, 0.3).expect("default drawing point is valid"),
                ],
                black,
                stroke,
            )
            .expect("default freehand drawing is valid"),
        )),
    }
}

fn required_path(value: &str) -> Result<ProjectPath, String> {
    if value.trim().is_empty() {
        return Err("Enter a project-relative file path first".to_owned());
    }
    project_path(value)
}

fn project_path(value: &str) -> Result<ProjectPath, String> {
    ProjectPath::new(value.trim()).map_err(|error| error.to_string())
}

fn label(kind: Kind) -> &'static str {
    match kind {
        Kind::Note => "Note",
        Kind::FileTree => "File tree",
        Kind::Artifact => "Artifact",
        Kind::Diff => "Diff",
        Kind::Text => "Text",
        Kind::Rectangle => "Rectangle",
        Kind::Ellipse => "Ellipse",
        Kind::Arrow => "Arrow",
        Kind::Freehand => "Freehand",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use openpodium::domain::{
        Agent, AgentId, CanvasLayout, Connection, ConnectionId, ConnectionKind, DomainCommand,
        NodeId, NodeTarget, WorkspaceId,
    };

    use super::*;

    #[test]
    fn connected_notes_resolve_only_direct_note_neighbors() {
        let checkout = tempfile::tempdir().unwrap();
        fs::create_dir_all(checkout.path().join(".openpodium/notes")).unwrap();
        fs::write(
            checkout.path().join(".openpodium/notes/context.md"),
            "Context",
        )
        .unwrap();
        let mut workspace = Workspace::new(WorkspaceId::new(1), Name::new("Test").unwrap());
        let agent_node = Node::new(
            NodeId::new(1),
            NodeTarget::Agent(AgentId::new(1)),
            CanvasPoint::new(0.0, 0.0).unwrap(),
            CanvasSize::new(320.0, 240.0).unwrap(),
        );
        workspace
            .execute(DomainCommand::AddAgentNode {
                agent: Agent::new(AgentId::new(1), Name::new("Agent").unwrap(), None),
                node: agent_node,
            })
            .unwrap();
        let note = Node::with_content(
            NodeId::new(2),
            CanvasNodeContent::Note {
                path: ProjectPath::new(".openpodium/notes/context.md").unwrap(),
                title: Name::new("Context").unwrap(),
            },
            CanvasPoint::new(400.0, 0.0).unwrap(),
            CanvasSize::new(320.0, 240.0).unwrap(),
        );
        workspace.execute(DomainCommand::AddNode(note)).unwrap();
        let before = workspace.canvas_layout();
        let after = CanvasLayout::new(
            before.nodes().to_vec(),
            vec![],
            vec![Connection::new(
                ConnectionId::new(1),
                NodeId::new(1),
                NodeId::new(2),
                ConnectionKind::Reference,
            )],
        );
        workspace
            .execute(DomainCommand::ReplaceCanvas { before, after })
            .unwrap();

        assert_eq!(
            connected_note_paths(&workspace, NodeId::new(1), checkout.path()),
            vec![
                dunce::canonicalize(checkout.path().join(".openpodium/notes/context.md")).unwrap()
            ]
        );
    }
}
