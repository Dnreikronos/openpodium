//! Versioned, SQLite-independent documents for sharing workspace designs.
//!
//! Portable documents deliberately use document-local symbolic identifiers.  A
//! pasted or imported document therefore cannot overwrite an entity merely
//! because its source happened to use the same numeric identifier.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::codec::CanvasLayoutV1;
use crate::domain::{
    Agent, AgentId, AgentProgram, Arrow, CanvasColor, CanvasLayout, CanvasNodeContent, CanvasPoint,
    CanvasSize, CanvasText, Connection, ConnectionId, ConnectionKind, Content, DiffComparison,
    DomainCommand, Handoff, HandoffId, HandoffPayload, Name, Node, NodeGroup, NodeGroupId, NodeId,
    NodeTarget, NormalizedPoint, PortalConfig, PortalPresentation, PortalTarget, PortalTargetKind,
    ProjectPath, Role, RoleColor, RoleIcon, RoleId, Shape, ShapeKind, StrokeWidth, Task, TaskId,
    TaskState, Workspace, WorkspaceIcon,
};

const TEMPLATE_FORMAT: &str = "openpodium-template";
const ARCHIVE_FORMAT: &str = "openpodium-workspace-archive";
const DOCUMENT_VERSION: u32 = 2;
const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
const MAX_ITEMS: usize = 8_192;
const MAX_ID_CHARS: usize = 128;

/// The portable template document.  The fields are public for callers that
/// need to inspect a preview, while constructors should normally be obtained
/// through [`export_template`] and [`decode_template`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateDocumentV1 {
    pub format: String,
    pub version: u32,
    pub template: TemplateBodyV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateBodyV1 {
    pub origin: PointV1,
    pub roles: Vec<RoleV1>,
    pub agents: Vec<AgentV1>,
    pub tasks: Vec<TaskV1>,
    pub handoffs: Vec<HandoffV1>,
    pub canvas: CanvasV1,
}

/// A workspace archive contains only durable, portable workspace data.  In
/// particular, it has no workspace directory, floor, environment, attachment,
/// terminal, or runtime-state field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceArchiveV1 {
    pub format: String,
    pub version: u32,
    pub archive: WorkspaceArchiveBodyV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceArchiveBodyV1 {
    pub settings: PortableSettingsV1,
    pub roles: Vec<RoleV1>,
    pub agents: Vec<AgentV1>,
    pub tasks: Vec<TaskV1>,
    pub handoffs: Vec<HandoffV1>,
    pub canvas: CanvasV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableSettingsV1 {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointV1 {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleV1 {
    pub id: String,
    pub name: String,
    pub color: String,
    pub icon: String,
    pub instructions: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentV1 {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_id: Option<String>,
    pub program: AgentProgramV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentProgramV1 {
    Codex,
    Claude,
    OpenCode,
    Shell,
    Custom { launcher_id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskV1 {
    pub id: String,
    pub title: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_source_state: Option<RetrySourceStateV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrySourceStateV1 {
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffV1 {
    pub id: String,
    pub source: String,
    pub recipient: String,
    pub payload: HandoffPayloadV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HandoffPayloadV1 {
    Task { task_id: String },
    Question { content: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanvasV1 {
    pub nodes: Vec<CanvasNodeV1>,
    pub groups: Vec<NodeGroupV1>,
    pub connections: Vec<ConnectionV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanvasNodeV1 {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<NodeTargetV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<CanvasContentV1>,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub z_index: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeGroupV1 {
    pub id: String,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionV1 {
    pub id: String,
    pub source: String,
    pub target: String,
    pub kind: ConnectionKindV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionKindV1 {
    Coordination,
    Assignment,
    Dependency,
    Handoff,
    Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum NodeTargetV1 {
    Agent { id: String },
    Task { id: String },
    Handoff { id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CanvasContentV1 {
    Note {
        path: String,
        title: String,
    },
    FileTree {
        root: String,
    },
    Artifact {
        path: String,
    },
    Diff {
        path: String,
        comparison: DiffComparisonV1,
    },
    Text {
        markdown: String,
    },
    Portal {
        kind: PortalTargetKindV1,
        selector: String,
        preserve_aspect_ratio: bool,
        frame_rate_limit: u16,
    },
    Shape {
        shape: ShapeV1,
    },
    Arrow {
        arrow: ArrowV1,
    },
    Freehand {
        freehand: FreehandV1,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortalTargetKindV1 {
    Browser,
    Android,
    Ios,
}

impl From<PortalTargetKind> for PortalTargetKindV1 {
    fn from(kind: PortalTargetKind) -> Self {
        match kind {
            PortalTargetKind::Browser => Self::Browser,
            PortalTargetKind::Android => Self::Android,
            PortalTargetKind::Ios => Self::Ios,
        }
    }
}

impl From<PortalTargetKindV1> for PortalTargetKind {
    fn from(kind: PortalTargetKindV1) -> Self {
        match kind {
            PortalTargetKindV1::Browser => Self::Browser,
            PortalTargetKindV1::Android => Self::Android,
            PortalTargetKindV1::Ios => Self::Ios,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffComparisonV1 {
    WorkingTreeAgainstHead,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShapeV1 {
    pub kind: ShapeKindV1,
    pub fill: [u8; 4],
    pub stroke: [u8; 4],
    pub stroke_width: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArrowV1 {
    pub start: PointV1,
    pub end: PointV1,
    pub stroke: [u8; 4],
    pub stroke_width: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreehandV1 {
    pub points: Vec<PointV1>,
    pub stroke: [u8; 4],
    pub stroke_width: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKindV1 {
    Rectangle,
    Ellipse,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SecretWarning {
    pub field: String,
    pub pattern: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportCounts {
    pub roles: usize,
    pub agents: usize,
    pub tasks: usize,
    pub handoffs: usize,
    pub nodes: usize,
    pub groups: usize,
    pub connections: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPreview {
    pub counts: ImportCounts,
    pub referenced_paths: Vec<String>,
    pub unresolved_launchers: Vec<String>,
    pub role_conflicts: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PortableImport {
    pub commands: Vec<DomainCommand>,
    pub preview: ImportPreview,
    /// The agent each symbolic document identifier became. Callers that must
    /// keep a durable binding to an instantiated arrangement — a routine run,
    /// for one — need this to follow the fresh identifiers.
    pub agents: BTreeMap<String, AgentId>,
}

pub fn export_template(
    workspace: &Workspace,
    selection: &[NodeId],
) -> Result<String, PortableError> {
    let layout = workspace.canvas_layout();
    let selected = expanded_selection(&layout, selection);
    if selected.is_empty() {
        return Err(PortableError::EmptySelection);
    }

    let layout = CanvasLayout::new(
        layout
            .nodes()
            .iter()
            .filter(|node| selected.contains(&node.id()))
            .cloned()
            .collect(),
        layout
            .groups()
            .iter()
            .filter(|group| group.members().all(|id| selected.contains(&id)))
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
    );
    let portable_handoffs = workspace
        .handoffs()
        .filter(|handoff| handoff.source().is_some())
        .map(|handoff| handoff.id())
        .collect();
    let layout = without_handoff_nodes(&layout, &portable_handoffs);
    let origin = layout
        .nodes()
        .iter()
        .map(Node::position)
        .map(PointV1::from)
        .reduce(|left, right| PointV1 {
            x: left.x.min(right.x),
            y: left.y.min(right.y),
        })
        .ok_or(PortableError::EmptySelection)?;

    let references = referenced_entities(workspace, &layout)?;
    let body = build_body(workspace, &layout, references, Some(origin))?;
    let warnings = scan_template(&body);
    if !warnings.is_empty() {
        return Err(PortableError::SecretDetected(warnings));
    }
    encode(&TemplateDocumentV1 {
        format: TEMPLATE_FORMAT.to_owned(),
        version: DOCUMENT_VERSION,
        template: body,
    })
}

pub fn decode_template(payload: &str) -> Result<TemplateDocumentV1, PortableError> {
    let document = decode_versioned(payload.as_bytes(), TEMPLATE_FORMAT, |value| {
        serde_json::from_value(value).map_err(PortableError::InvalidJson)
    })?;
    validate_template(&document)?;
    let warnings = scan_template(&document.template);
    if !warnings.is_empty() {
        return Err(PortableError::SecretDetected(warnings));
    }
    Ok(document)
}

pub fn export_workspace_archive(workspace: &Workspace) -> Result<String, PortableError> {
    // A handoff the routine scheduler submitted is an execution record of one
    // run, not reusable workspace structure: it names a run and step that the
    // destination has no counterpart for. The tasks it produced are ordinary
    // tasks and still travel, so nothing a person authored is lost.
    let handoffs: BTreeSet<_> = workspace
        .handoffs()
        .filter(|handoff| handoff.source().is_some())
        .map(|handoff| handoff.id())
        .collect();
    let layout = without_handoff_nodes(&workspace.all_canvas_layout(), &handoffs);
    let references = ReferencedEntities {
        roles: workspace.roles().map(|role| role.id()).collect(),
        agents: workspace.agents().map(|agent| agent.id()).collect(),
        tasks: workspace.tasks().map(|task| task.id()).collect(),
        handoffs,
    };
    let body = build_body(workspace, &layout, references, None)?;
    let archive = WorkspaceArchiveV1 {
        format: ARCHIVE_FORMAT.to_owned(),
        version: DOCUMENT_VERSION,
        archive: WorkspaceArchiveBodyV1 {
            settings: PortableSettingsV1 {
                name: workspace.settings().name().as_str().to_owned(),
                icon: workspace
                    .settings()
                    .icon()
                    .map(WorkspaceIcon::as_str)
                    .map(str::to_owned),
                instructions: workspace
                    .settings()
                    .instructions()
                    .map(Content::as_str)
                    .map(str::to_owned),
            },
            roles: body.roles,
            agents: body.agents,
            tasks: body.tasks,
            handoffs: body.handoffs,
            canvas: body.canvas,
        },
    };
    let payload = encode(&archive)?;
    let warnings = scan_archive(&archive);
    if !warnings.is_empty() {
        return Err(PortableError::SecretDetected(warnings));
    }
    Ok(payload)
}

pub fn decode_workspace_archive(payload: &str) -> Result<WorkspaceArchiveV1, PortableError> {
    let archive: WorkspaceArchiveV1 =
        decode_versioned(payload.as_bytes(), ARCHIVE_FORMAT, |value| {
            serde_json::from_value(value).map_err(PortableError::InvalidJson)
        })?;
    let warnings = scan_archive(&archive);
    if !warnings.is_empty() {
        return Err(PortableError::SecretDetected(warnings));
    }
    validate_body(
        &archive.archive.roles,
        &archive.archive.agents,
        &archive.archive.tasks,
        &archive.archive.handoffs,
        &archive.archive.canvas,
    )?;
    Ok(archive)
}

pub fn preview_template_import(
    document: &TemplateDocumentV1,
    workspace: &Workspace,
) -> Result<ImportPreview, PortableError> {
    validate_template(document)?;
    preview_body(
        &document.template.roles,
        &document.template.agents,
        &document.template.tasks,
        &document.template.handoffs,
        &document.template.canvas,
        workspace,
    )
}

pub fn preview_workspace_archive_import(
    archive: &WorkspaceArchiveV1,
    workspace: &Workspace,
) -> Result<ImportPreview, PortableError> {
    validate_archive(archive)?;
    preview_body(
        &archive.archive.roles,
        &archive.archive.agents,
        &archive.archive.tasks,
        &archive.archive.handoffs,
        &archive.archive.canvas,
        workspace,
    )
}

pub fn import_template(
    document: &TemplateDocumentV1,
    workspace: &Workspace,
    destination_origin: PointV1,
    launcher_mappings: &BTreeMap<String, crate::domain::CommandPresetId>,
) -> Result<PortableImport, PortableError> {
    import_template_with_mappings(
        document,
        workspace,
        destination_origin,
        launcher_mappings,
        &BTreeMap::new(),
    )
}

pub fn import_template_with_mappings(
    document: &TemplateDocumentV1,
    workspace: &Workspace,
    destination_origin: PointV1,
    launcher_mappings: &BTreeMap<String, crate::domain::CommandPresetId>,
    path_mappings: &BTreeMap<String, String>,
) -> Result<PortableImport, PortableError> {
    validate_template(document)?;
    let preview = preview_template_import(document, workspace)?;
    ensure_launchers_resolved(&preview, workspace, launcher_mappings)?;
    let (commands, agents) = build_commands(
        workspace,
        &document.template.roles,
        &document.template.agents,
        &document.template.tasks,
        &document.template.handoffs,
        &document.template.canvas,
        destination_origin,
        launcher_mappings,
        path_mappings,
        None,
    )?;
    Ok(PortableImport {
        commands,
        preview,
        agents,
    })
}

pub fn import_workspace_archive(
    archive: &WorkspaceArchiveV1,
    workspace: &Workspace,
    launcher_mappings: &BTreeMap<String, crate::domain::CommandPresetId>,
) -> Result<PortableImport, PortableError> {
    import_workspace_archive_with_mappings(archive, workspace, launcher_mappings, &BTreeMap::new())
}

pub fn import_workspace_archive_with_mappings(
    archive: &WorkspaceArchiveV1,
    workspace: &Workspace,
    launcher_mappings: &BTreeMap<String, crate::domain::CommandPresetId>,
    path_mappings: &BTreeMap<String, String>,
) -> Result<PortableImport, PortableError> {
    validate_archive(archive)?;
    let preview = preview_workspace_archive_import(archive, workspace)?;
    ensure_launchers_resolved(&preview, workspace, launcher_mappings)?;
    let (commands, agents) = build_commands(
        workspace,
        &archive.archive.roles,
        &archive.archive.agents,
        &archive.archive.tasks,
        &archive.archive.handoffs,
        &archive.archive.canvas,
        PointV1 { x: 0.0, y: 0.0 },
        launcher_mappings,
        path_mappings,
        Some(&archive.archive.settings),
    )?;
    Ok(PortableImport {
        commands,
        preview,
        agents,
    })
}

fn encode<T: Serialize>(document: &T) -> Result<String, PortableError> {
    let payload = serde_json::to_string_pretty(document).map_err(PortableError::InvalidJson)?;
    if payload.len() > MAX_DOCUMENT_BYTES {
        return Err(PortableError::SizeLimit {
            limit: MAX_DOCUMENT_BYTES,
        });
    }
    Ok(payload)
}

fn decode_versioned<T>(
    payload: &[u8],
    expected_format: &str,
    parse: impl FnOnce(Value) -> Result<T, PortableError>,
) -> Result<T, PortableError> {
    if payload.len() > MAX_DOCUMENT_BYTES {
        return Err(PortableError::SizeLimit {
            limit: MAX_DOCUMENT_BYTES,
        });
    }
    let mut value: Value = serde_json::from_slice(payload).map_err(PortableError::InvalidJson)?;
    let object = value.as_object_mut().ok_or_else(|| {
        PortableError::InvalidDocument("document must be a JSON object".to_owned())
    })?;
    let format = object
        .get("format")
        .and_then(Value::as_str)
        .ok_or_else(|| PortableError::InvalidDocument("document is missing format".to_owned()))?;
    if format != expected_format {
        return Err(PortableError::UnsupportedFormat(format.to_owned()));
    }
    let version = object
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| PortableError::InvalidDocument("document is missing version".to_owned()))?;
    match u32::try_from(version).unwrap_or(u32::MAX) {
        DOCUMENT_VERSION => parse(value),
        1 => {
            migrate_v1(object);
            parse(value)
        }
        0 => {
            migrate_v0(object, expected_format)?;
            parse(value)
        }
        found => Err(PortableError::UnsupportedVersion {
            found,
            supported: DOCUMENT_VERSION,
        }),
    }
}

fn migrate_v0(
    object: &mut serde_json::Map<String, Value>,
    format: &str,
) -> Result<(), PortableError> {
    object.insert("version".to_owned(), Value::from(DOCUMENT_VERSION));
    let body_name = if format == TEMPLATE_FORMAT {
        "template"
    } else {
        "archive"
    };
    let body = object
        .get_mut(body_name)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| {
            PortableError::InvalidDocument(format!("document is missing {body_name}"))
        })?;
    if format == TEMPLATE_FORMAT && !body.contains_key("origin") {
        body.insert(
            "origin".to_owned(),
            serde_json::json!({ "x": 0.0_f32, "y": 0.0_f32 }),
        );
    }
    Ok(())
}

fn migrate_v1(object: &mut serde_json::Map<String, Value>) {
    object.insert("version".to_owned(), Value::from(DOCUMENT_VERSION));
}

#[derive(Default)]
struct ReferencedEntities {
    roles: BTreeSet<RoleId>,
    agents: BTreeSet<AgentId>,
    tasks: BTreeSet<TaskId>,
    handoffs: BTreeSet<HandoffId>,
}

fn referenced_entities(
    workspace: &Workspace,
    layout: &CanvasLayout,
) -> Result<ReferencedEntities, PortableError> {
    let mut references = ReferencedEntities::default();
    let mut pending = layout
        .nodes()
        .iter()
        .filter_map(Node::reference)
        .collect::<Vec<_>>();
    while let Some(target) = pending.pop() {
        match target {
            NodeTarget::Agent(id) => {
                if references.agents.insert(id) {
                    let agent = workspace
                        .agent(id)
                        .ok_or_else(|| PortableError::MissingReference(format!("agent {id}")))?;
                    if let Some(role_id) = agent.role_id() {
                        workspace.role(role_id).ok_or_else(|| {
                            PortableError::MissingReference(format!("role {role_id}"))
                        })?;
                        references.roles.insert(role_id);
                    }
                }
            }
            NodeTarget::Task(id) => {
                if references.tasks.insert(id) {
                    let task = workspace
                        .task(id)
                        .ok_or_else(|| PortableError::MissingReference(format!("task {id}")))?;
                    if let Some(agent_id) = task.assignee() {
                        pending.push(NodeTarget::Agent(agent_id));
                    }
                    if let Some(retry_of) = task.retry_of() {
                        pending.push(NodeTarget::Task(retry_of));
                    }
                }
            }
            NodeTarget::Handoff(id) => {
                if references.handoffs.insert(id) {
                    let handoff = workspace
                        .handoff(id)
                        .ok_or_else(|| PortableError::MissingReference(format!("handoff {id}")))?;
                    pending.extend(handoff.source().map(NodeTarget::Agent));
                    pending.push(NodeTarget::Agent(handoff.recipient()));
                    if let HandoffPayload::Task(task_id) = handoff.payload() {
                        pending.push(NodeTarget::Task(*task_id));
                    }
                }
            }
        }
    }
    Ok(references)
}

/// Drops canvas nodes that point at a handoff the export is not carrying, and
/// the connections that touched them, so the document stays self-consistent.
fn without_handoff_nodes(layout: &CanvasLayout, kept: &BTreeSet<HandoffId>) -> CanvasLayout {
    let dropped: BTreeSet<NodeId> = layout
        .nodes()
        .iter()
        .filter(
            |node| matches!(node.reference(), Some(NodeTarget::Handoff(id)) if !kept.contains(&id)),
        )
        .map(Node::id)
        .collect();
    if dropped.is_empty() {
        return layout.clone();
    }
    CanvasLayout::new(
        layout
            .nodes()
            .iter()
            .filter(|node| !dropped.contains(&node.id()))
            .cloned()
            .collect(),
        layout
            .groups()
            .iter()
            .filter(|group| group.members().all(|node| !dropped.contains(&node)))
            .cloned()
            .collect(),
        layout
            .connections()
            .iter()
            .filter(|connection| {
                !dropped.contains(&connection.source()) && !dropped.contains(&connection.target())
            })
            .cloned()
            .collect(),
    )
}

fn build_body(
    workspace: &Workspace,
    layout: &CanvasLayout,
    references: ReferencedEntities,
    origin: Option<PointV1>,
) -> Result<TemplateBodyV1, PortableError> {
    let origin = origin.unwrap_or(PointV1 { x: 0.0, y: 0.0 });
    Ok(TemplateBodyV1 {
        origin,
        roles: workspace
            .roles()
            .filter(|role| references.roles.contains(&role.id()))
            .map(role_record)
            .collect(),
        agents: workspace
            .agents()
            .filter(|agent| references.agents.contains(&agent.id()))
            .map(agent_record)
            .collect(),
        tasks: workspace
            .tasks()
            .filter(|task| references.tasks.contains(&task.id()))
            .map(|task| task_record(workspace, task))
            .collect::<Result<_, _>>()?,
        handoffs: workspace
            .handoffs()
            .filter(|handoff| references.handoffs.contains(&handoff.id()))
            .map(handoff_record)
            .collect::<Result<_, _>>()?,
        canvas: canvas_record(layout, origin)?,
    })
}

fn role_record(role: &Role) -> RoleV1 {
    RoleV1 {
        id: symbolic("role", role.id().get()),
        name: role.name().as_str().to_owned(),
        color: role.color().as_str().to_owned(),
        icon: role.icon().as_str().to_owned(),
        instructions: role.instructions().as_str().to_owned(),
    }
}

fn agent_record(agent: &Agent) -> AgentV1 {
    let program = match agent.program() {
        AgentProgram::Codex => AgentProgramV1::Codex,
        AgentProgram::Claude => AgentProgramV1::Claude,
        AgentProgram::OpenCode => AgentProgramV1::OpenCode,
        AgentProgram::Shell => AgentProgramV1::Shell,
        AgentProgram::Custom(id) => AgentProgramV1::Custom {
            launcher_id: symbolic("launcher", id.get()),
        },
    };
    AgentV1 {
        id: symbolic("agent", agent.id().get()),
        name: agent.name().as_str().to_owned(),
        role_id: agent.role_id().map(|id| symbolic("role", id.get())),
        program,
    }
}

fn task_record(workspace: &Workspace, task: &Task) -> Result<TaskV1, PortableError> {
    let retry_source_state = task
        .retry_of()
        .map(|retry_of| {
            workspace
                .task(retry_of)
                .ok_or_else(|| PortableError::MissingReference(format!("task {retry_of}")))
                .and_then(|source| match source.state() {
                    TaskState::Failed => Ok(RetrySourceStateV1::Failed),
                    TaskState::Cancelled => Ok(RetrySourceStateV1::Cancelled),
                    state => Err(PortableError::InvalidDocument(format!(
                        "task {} has retry source {} in unsupported state {state}",
                        task.id(),
                        retry_of
                    ))),
                })
        })
        .transpose()?;
    Ok(TaskV1 {
        id: symbolic("task", task.id().get()),
        title: task.title().as_str().to_owned(),
        prompt: task.prompt().as_str().to_owned(),
        assignee: task.assignee().map(|id| symbolic("agent", id.get())),
        retry_of: task.retry_of().map(|id| symbolic("task", id.get())),
        retry_source_state,
    })
}

fn handoff_record(handoff: &Handoff) -> Result<HandoffV1, PortableError> {
    let payload = match handoff.payload() {
        HandoffPayload::Task(id) => HandoffPayloadV1::Task {
            task_id: symbolic("task", id.get()),
        },
        HandoffPayload::Question(content) => HandoffPayloadV1::Question {
            content: content.as_str().to_owned(),
        },
    };
    // Routine-submitted handoffs are execution records, not reusable canvas
    // structure, and callers filter them out before they reach this point.
    let source = handoff.source().ok_or_else(|| {
        PortableError::InvalidDocument(format!(
            "handoff {} was submitted by a routine and cannot be exported",
            handoff.id()
        ))
    })?;
    Ok(HandoffV1 {
        id: symbolic("handoff", handoff.id().get()),
        source: symbolic("agent", source.get()),
        recipient: symbolic("agent", handoff.recipient().get()),
        payload,
    })
}

fn canvas_record(layout: &CanvasLayout, origin: PointV1) -> Result<CanvasV1, PortableError> {
    // Keep portable canvas validation aligned with the durable canvas DTO,
    // while the document below replaces its numeric IDs with symbolic ones.
    CanvasLayoutV1::from(layout)
        .into_domain()
        .map_err(PortableError::InvalidDocument)?;
    Ok(CanvasV1 {
        nodes: layout
            .nodes()
            .iter()
            .map(|node| canvas_node_record(node, origin))
            .collect::<Result<_, _>>()?,
        groups: layout
            .groups()
            .iter()
            .map(|group| NodeGroupV1 {
                id: symbolic("group", group.id().get()),
                members: group
                    .members()
                    .map(|id| symbolic("node", id.get()))
                    .collect(),
            })
            .collect(),
        connections: layout
            .connections()
            .iter()
            .map(|connection| ConnectionV1 {
                id: symbolic("connection", connection.id().get()),
                source: symbolic("node", connection.source().get()),
                target: symbolic("node", connection.target().get()),
                kind: connection.kind().into(),
            })
            .collect(),
    })
}

fn canvas_node_record(node: &Node, origin: PointV1) -> Result<CanvasNodeV1, PortableError> {
    let (target, content) = match node.content() {
        CanvasNodeContent::Reference(target) => (Some((*target).into()), None),
        content => (None, Some(content_record(content)?)),
    };
    Ok(CanvasNodeV1 {
        id: symbolic("node", node.id().get()),
        target,
        content,
        x: node.position().x() - origin.x,
        y: node.position().y() - origin.y,
        width: node.size().width(),
        height: node.size().height(),
        z_index: node.z_index(),
    })
}

fn content_record(content: &CanvasNodeContent) -> Result<CanvasContentV1, PortableError> {
    Ok(match content {
        CanvasNodeContent::Reference(_) => {
            return Err(PortableError::InvalidDocument(
                "reference content must be encoded as target".to_owned(),
            ));
        }
        CanvasNodeContent::Note { path, title } => CanvasContentV1::Note {
            path: path.as_str().to_owned(),
            title: title.as_str().to_owned(),
        },
        CanvasNodeContent::FileTree { root } => CanvasContentV1::FileTree {
            root: root.as_str().to_owned(),
        },
        CanvasNodeContent::Artifact { path } => CanvasContentV1::Artifact {
            path: path.as_str().to_owned(),
        },
        CanvasNodeContent::Diff { path, comparison } => CanvasContentV1::Diff {
            path: path.as_str().to_owned(),
            comparison: (*comparison).into(),
        },
        CanvasNodeContent::Text { markdown } => CanvasContentV1::Text {
            markdown: markdown.as_str().to_owned(),
        },
        CanvasNodeContent::Portal(config) => CanvasContentV1::Portal {
            kind: config.target().kind().into(),
            selector: config.target().selector().to_owned(),
            preserve_aspect_ratio: config.presentation().preserve_aspect_ratio(),
            frame_rate_limit: config.presentation().frame_rate_limit(),
        },
        CanvasNodeContent::Shape(shape) => CanvasContentV1::Shape {
            shape: ShapeV1 {
                kind: shape.kind().into(),
                fill: shape.fill().channels(),
                stroke: shape.stroke().channels(),
                stroke_width: shape.stroke_width().get(),
            },
        },
        CanvasNodeContent::Arrow(arrow) => CanvasContentV1::Arrow {
            arrow: ArrowV1 {
                start: arrow.start().into(),
                end: arrow.end().into(),
                stroke: arrow.stroke().channels(),
                stroke_width: arrow.stroke_width().get(),
                label: arrow.label().map(|label| label.as_str().to_owned()),
            },
        },
        CanvasNodeContent::Freehand(freehand) => CanvasContentV1::Freehand {
            freehand: FreehandV1 {
                points: freehand.points().iter().copied().map(Into::into).collect(),
                stroke: freehand.stroke().channels(),
                stroke_width: freehand.stroke_width().get(),
            },
        },
    })
}

#[allow(clippy::too_many_arguments)]
fn build_commands(
    workspace: &Workspace,
    roles: &[RoleV1],
    agents: &[AgentV1],
    tasks: &[TaskV1],
    handoffs: &[HandoffV1],
    canvas: &CanvasV1,
    origin: PointV1,
    launcher_mappings: &BTreeMap<String, crate::domain::CommandPresetId>,
    path_mappings: &BTreeMap<String, String>,
    settings: Option<&PortableSettingsV1>,
) -> Result<(Vec<DomainCommand>, BTreeMap<String, AgentId>), PortableError> {
    let mut commands = Vec::new();
    let mut role_ids = IdAllocator::new(workspace.roles().map(|role| role.id().get()));
    let mut agent_ids = IdAllocator::new(workspace.agents().map(|agent| agent.id().get()));
    let mut task_ids = IdAllocator::new(workspace.tasks().map(|task| task.id().get()));
    let mut handoff_ids = IdAllocator::new(workspace.handoffs().map(|handoff| handoff.id().get()));
    let mut node_ids = IdAllocator::new(
        workspace
            .all_canvas_layout()
            .nodes()
            .iter()
            .map(|n| n.id().get()),
    );
    let mut group_ids = IdAllocator::new(
        workspace
            .all_canvas_layout()
            .groups()
            .iter()
            .map(|g| g.id().get()),
    );
    let mut connection_ids = IdAllocator::new(
        workspace
            .all_canvas_layout()
            .connections()
            .iter()
            .map(|c| c.id().get()),
    );
    let mut role_map = BTreeMap::new();
    let mut agent_map = BTreeMap::new();
    let mut task_map = BTreeMap::new();
    let mut handoff_map = BTreeMap::new();

    if let Some(settings) = settings {
        let imported_settings = crate::domain::WorkspaceSettings::new(
            Name::new(settings.name.clone())
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?,
            settings
                .icon
                .as_ref()
                .map(|icon| WorkspaceIcon::new(icon.clone()))
                .transpose()
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?,
            workspace.settings().working_directory().cloned(),
            settings
                .instructions
                .as_ref()
                .map(|instructions| Content::new(instructions.clone()))
                .transpose()
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?,
        );
        if &imported_settings != workspace.settings() {
            commands.push(DomainCommand::UpdateWorkspaceSettings(imported_settings));
        }
    }

    for role in roles {
        let id = role_ids.allocate();
        role_map.insert(role.id.clone(), RoleId::new(id));
        commands.push(DomainCommand::AddRole(Role::with_appearance(
            RoleId::new(id),
            Name::new(role.name.clone())
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?,
            RoleColor::new(role.color.clone())
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?,
            RoleIcon::new(role.icon.clone())
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?,
            Content::new(role.instructions.clone())
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?,
        )));
    }
    for agent in agents {
        let id = agent_ids.allocate();
        let role_id = agent
            .role_id
            .as_ref()
            .map(|role| {
                role_map
                    .get(role)
                    .copied()
                    .ok_or_else(|| PortableError::MissingReference(role.clone()))
            })
            .transpose()?;
        let program = match &agent.program {
            AgentProgramV1::Codex => AgentProgram::Codex,
            AgentProgramV1::Claude => AgentProgram::Claude,
            AgentProgramV1::OpenCode => AgentProgram::OpenCode,
            AgentProgramV1::Shell => AgentProgram::Shell,
            AgentProgramV1::Custom { launcher_id } => AgentProgram::Custom(
                *launcher_mappings
                    .get(launcher_id)
                    .ok_or_else(|| PortableError::UnresolvedLaunchers(vec![launcher_id.clone()]))?,
            ),
        };
        agent_map.insert(agent.id.clone(), AgentId::new(id));
        commands.push(DomainCommand::AddAgent(Agent::with_program(
            AgentId::new(id),
            Name::new(agent.name.clone())
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?,
            role_id,
            program,
        )));
    }
    let task_by_id = tasks
        .iter()
        .map(|task| (task.id.clone(), task))
        .collect::<BTreeMap<_, _>>();
    for task in tasks {
        task_map.insert(task.id.clone(), TaskId::new(task_ids.allocate()));
    }
    let mut retry_source_states = BTreeMap::new();
    for task in tasks {
        let Some(retry_of) = &task.retry_of else {
            if task.retry_source_state.is_some() {
                return Err(PortableError::InvalidDocument(format!(
                    "task {} has retry source state without a retry predecessor",
                    task.id
                )));
            }
            continue;
        };
        if !task_map.contains_key(retry_of) {
            return Err(PortableError::MissingReference(retry_of.clone()));
        }
        let source_state = task
            .retry_source_state
            .unwrap_or(RetrySourceStateV1::Failed);
        if let Some(previous) = retry_source_states.insert(retry_of.clone(), source_state)
            && previous != source_state
        {
            return Err(PortableError::InvalidDocument(format!(
                "retry predecessor {retry_of} has conflicting terminal states"
            )));
        }
    }
    let mut task_order = Vec::with_capacity(tasks.len());
    let mut task_marks = BTreeMap::new();
    for task in tasks {
        append_task_order(
            task.id.as_str(),
            &task_by_id,
            &mut task_marks,
            &mut task_order,
        )?;
    }
    for task_id in task_order {
        let task = task_by_id
            .get(&task_id)
            .copied()
            .expect("task order contains known task");
        let task_id = *task_map
            .get(&task.id)
            .expect("task map contains known task");
        let assignee = task
            .assignee
            .as_ref()
            .map(|agent| {
                agent_map
                    .get(agent)
                    .copied()
                    .ok_or_else(|| PortableError::MissingReference(agent.clone()))
            })
            .transpose()?;
        let retry_of = task
            .retry_of
            .as_ref()
            .map(|retry| {
                task_map
                    .get(retry)
                    .copied()
                    .ok_or_else(|| PortableError::MissingReference(retry.clone()))
            })
            .transpose()?;
        commands.push(DomainCommand::AddTask(Task::new(
            task_id,
            Name::new(task.title.clone())
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?,
            Content::new(task.prompt.clone())
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?,
            assignee,
            retry_of,
        )));
        if let Some(source_state) = retry_source_states.get(&task.id) {
            commands.push(DomainCommand::TransitionTask {
                task_id,
                to: match source_state {
                    RetrySourceStateV1::Failed => TaskState::Failed,
                    RetrySourceStateV1::Cancelled => TaskState::Cancelled,
                },
            });
        }
    }
    for handoff in handoffs {
        let id = handoff_ids.allocate();
        let source = agent_map
            .get(&handoff.source)
            .copied()
            .ok_or_else(|| PortableError::MissingReference(handoff.source.clone()))?;
        let recipient = agent_map
            .get(&handoff.recipient)
            .copied()
            .ok_or_else(|| PortableError::MissingReference(handoff.recipient.clone()))?;
        let payload = match &handoff.payload {
            HandoffPayloadV1::Task { task_id } => HandoffPayload::Task(
                *task_map
                    .get(task_id)
                    .ok_or_else(|| PortableError::MissingReference(task_id.clone()))?,
            ),
            HandoffPayloadV1::Question { content } => HandoffPayload::Question(
                Content::new(content.clone())
                    .map_err(|error| PortableError::InvalidDomain(error.to_string()))?,
            ),
        };
        handoff_map.insert(handoff.id.clone(), HandoffId::new(id));
        commands.push(DomainCommand::AddHandoff(Handoff::new(
            HandoffId::new(id),
            source,
            recipient,
            payload,
        )));
    }

    let imported = canvas_domain(
        canvas,
        origin,
        &role_map,
        &agent_map,
        &task_map,
        &handoff_map,
        path_mappings,
        &mut node_ids,
        &mut group_ids,
        &mut connection_ids,
    )?;
    let current = workspace.canvas_layout();
    let combined = CanvasLayout::new(
        current
            .nodes()
            .iter()
            .cloned()
            .chain(imported.nodes().iter().cloned())
            .collect(),
        current
            .groups()
            .iter()
            .cloned()
            .chain(imported.groups().iter().cloned())
            .collect(),
        current
            .connections()
            .iter()
            .cloned()
            .chain(imported.connections().iter().cloned())
            .collect(),
    );
    commands.push(DomainCommand::ReplaceCanvas {
        before: workspace.canvas_layout(),
        after: combined,
    });
    Ok((commands, agent_map))
}

fn append_task_order(
    id: &str,
    tasks: &BTreeMap<String, &TaskV1>,
    marks: &mut BTreeMap<String, u8>,
    order: &mut Vec<String>,
) -> Result<(), PortableError> {
    match marks.get(id).copied() {
        Some(2) => return Ok(()),
        Some(1) => {
            return Err(PortableError::InvalidDocument(format!(
                "task retry relationship contains a cycle at {id}"
            )));
        }
        _ => {}
    }
    let task = tasks
        .get(id)
        .copied()
        .ok_or_else(|| PortableError::MissingReference(id.to_owned()))?;
    marks.insert(id.to_owned(), 1);
    if let Some(retry_of) = &task.retry_of {
        append_task_order(retry_of, tasks, marks, order)?;
    }
    marks.insert(id.to_owned(), 2);
    order.push(id.to_owned());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn canvas_domain(
    canvas: &CanvasV1,
    origin: PointV1,
    _roles: &BTreeMap<String, RoleId>,
    agents: &BTreeMap<String, AgentId>,
    tasks: &BTreeMap<String, TaskId>,
    handoffs: &BTreeMap<String, HandoffId>,
    path_mappings: &BTreeMap<String, String>,
    node_ids: &mut IdAllocator,
    group_ids: &mut IdAllocator,
    connection_ids: &mut IdAllocator,
) -> Result<CanvasLayout, PortableError> {
    let mut nodes = Vec::new();
    let mut node_map = BTreeMap::new();
    for node in &canvas.nodes {
        let id = NodeId::new(node_ids.allocate());
        node_map.insert(node.id.clone(), id);
        let content = match (&node.target, &node.content) {
            (Some(target), None) => {
                CanvasNodeContent::Reference(target_domain(target, agents, tasks, handoffs)?)
            }
            (None, Some(content)) => content_domain(content, path_mappings)?,
            _ => {
                return Err(PortableError::InvalidDocument(format!(
                    "node {} has invalid content",
                    node.id
                )));
            }
        };
        nodes.push(Node::with_content_and_z_index(
            id,
            content,
            CanvasPoint::new(node.x + origin.x, node.y + origin.y)
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?,
            CanvasSize::new(node.width, node.height)
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?,
            node.z_index,
        ));
    }
    let groups = canvas
        .groups
        .iter()
        .map(|group| {
            let members = group
                .members
                .iter()
                .map(|id| {
                    node_map
                        .get(id)
                        .copied()
                        .ok_or_else(|| PortableError::MissingReference(id.clone()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(NodeGroup::new(
                NodeGroupId::new(group_ids.allocate()),
                members,
            ))
        })
        .collect::<Result<Vec<_>, PortableError>>()?;
    let connections = canvas
        .connections
        .iter()
        .map(|connection| {
            let source = node_map
                .get(&connection.source)
                .copied()
                .ok_or_else(|| PortableError::MissingReference(connection.source.clone()))?;
            let target = node_map
                .get(&connection.target)
                .copied()
                .ok_or_else(|| PortableError::MissingReference(connection.target.clone()))?;
            Ok(Connection::new(
                ConnectionId::new(connection_ids.allocate()),
                source,
                target,
                connection.kind.into(),
            ))
        })
        .collect::<Result<Vec<_>, PortableError>>()?;
    Ok(CanvasLayout::new(nodes, groups, connections))
}

fn target_domain(
    target: &NodeTargetV1,
    agents: &BTreeMap<String, AgentId>,
    tasks: &BTreeMap<String, TaskId>,
    handoffs: &BTreeMap<String, HandoffId>,
) -> Result<NodeTarget, PortableError> {
    match target {
        NodeTargetV1::Agent { id } => agents.get(id).copied().map(NodeTarget::Agent),
        NodeTargetV1::Task { id } => tasks.get(id).copied().map(NodeTarget::Task),
        NodeTargetV1::Handoff { id } => handoffs.get(id).copied().map(NodeTarget::Handoff),
    }
    .ok_or_else(|| PortableError::MissingReference(target_id(target).to_owned()))
}

fn target_id(target: &NodeTargetV1) -> &str {
    match target {
        NodeTargetV1::Agent { id } | NodeTargetV1::Task { id } | NodeTargetV1::Handoff { id } => id,
    }
}

fn content_domain(
    content: &CanvasContentV1,
    path_mappings: &BTreeMap<String, String>,
) -> Result<CanvasNodeContent, PortableError> {
    Ok(match content {
        CanvasContentV1::Note { path, title } => CanvasNodeContent::Note {
            path: mapped_project_path(path, path_mappings)?,
            title: name(title)?,
        },
        CanvasContentV1::FileTree { root } => CanvasNodeContent::FileTree {
            root: mapped_project_path(root, path_mappings)?,
        },
        CanvasContentV1::Artifact { path } => CanvasNodeContent::Artifact {
            path: mapped_project_path(path, path_mappings)?,
        },
        CanvasContentV1::Diff { path, comparison } => CanvasNodeContent::Diff {
            path: mapped_project_path(path, path_mappings)?,
            comparison: (*comparison).into(),
        },
        CanvasContentV1::Text { markdown } => CanvasNodeContent::Text {
            markdown: CanvasText::new(markdown.clone())
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?,
        },
        CanvasContentV1::Portal {
            kind,
            selector,
            preserve_aspect_ratio,
            frame_rate_limit,
        } => {
            let target = PortalTarget::new((*kind).into(), selector.clone())
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?;
            let presentation = PortalPresentation::new(*preserve_aspect_ratio, *frame_rate_limit)
                .ok_or_else(|| {
                PortableError::InvalidDomain(
                    "portal frame rate limit must be greater than zero".to_owned(),
                )
            })?;
            CanvasNodeContent::Portal(PortalConfig::new(target, presentation))
        }
        CanvasContentV1::Shape { shape } => CanvasNodeContent::Shape(Shape::new(
            shape.kind.into(),
            CanvasColor::rgba(shape.fill[0], shape.fill[1], shape.fill[2], shape.fill[3]),
            CanvasColor::rgba(
                shape.stroke[0],
                shape.stroke[1],
                shape.stroke[2],
                shape.stroke[3],
            ),
            StrokeWidth::new(shape.stroke_width)
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?,
        )),
        CanvasContentV1::Arrow { arrow } => CanvasNodeContent::Arrow(Arrow::new(
            normalized_point(arrow.start)?,
            normalized_point(arrow.end)?,
            CanvasColor::rgba(
                arrow.stroke[0],
                arrow.stroke[1],
                arrow.stroke[2],
                arrow.stroke[3],
            ),
            StrokeWidth::new(arrow.stroke_width)
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?,
            arrow.label.as_ref().map(|label| name(label)).transpose()?,
        )),
        CanvasContentV1::Freehand { freehand } => CanvasNodeContent::Freehand(
            crate::domain::Freehand::new(
                freehand
                    .points
                    .iter()
                    .copied()
                    .map(normalized_point)
                    .collect::<Result<_, _>>()?,
                CanvasColor::rgba(
                    freehand.stroke[0],
                    freehand.stroke[1],
                    freehand.stroke[2],
                    freehand.stroke[3],
                ),
                StrokeWidth::new(freehand.stroke_width)
                    .map_err(|error| PortableError::InvalidDomain(error.to_string()))?,
            )
            .map_err(|error| PortableError::InvalidDomain(error.to_string()))?,
        ),
    })
}

fn project_path(value: &str) -> Result<ProjectPath, PortableError> {
    ProjectPath::new(value.to_owned())
        .map_err(|error| PortableError::InvalidDomain(error.to_string()))
}

fn mapped_project_path(
    source: &str,
    path_mappings: &BTreeMap<String, String>,
) -> Result<ProjectPath, PortableError> {
    project_path(
        path_mappings
            .get(source)
            .map(String::as_str)
            .unwrap_or(source),
    )
}

fn normalized_point(value: PointV1) -> Result<NormalizedPoint, PortableError> {
    NormalizedPoint::new(value.x, value.y)
        .map_err(|error| PortableError::InvalidDomain(error.to_string()))
}

fn name(value: &str) -> Result<Name, PortableError> {
    Name::new(value.to_owned()).map_err(|error| PortableError::InvalidDomain(error.to_string()))
}

fn preview_body(
    roles: &[RoleV1],
    agents: &[AgentV1],
    tasks: &[TaskV1],
    handoffs: &[HandoffV1],
    canvas: &CanvasV1,
    workspace: &Workspace,
) -> Result<ImportPreview, PortableError> {
    let mut referenced_paths = BTreeSet::new();
    for node in &canvas.nodes {
        if let Some(content) = &node.content {
            match content {
                CanvasContentV1::Note { path, .. }
                | CanvasContentV1::Artifact { path }
                | CanvasContentV1::Diff { path, .. } => {
                    referenced_paths.insert(path.clone());
                }
                CanvasContentV1::FileTree { root } => {
                    referenced_paths.insert(root.clone());
                }
                CanvasContentV1::Text { .. }
                | CanvasContentV1::Portal { .. }
                | CanvasContentV1::Shape { .. }
                | CanvasContentV1::Arrow { .. }
                | CanvasContentV1::Freehand { .. } => {}
            }
        }
    }
    let unresolved_launchers = agents
        .iter()
        .filter_map(|agent| match &agent.program {
            AgentProgramV1::Custom { launcher_id } => Some(launcher_id.clone()),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let imported_role_names = roles
        .iter()
        .map(|role| role.name.as_str())
        .collect::<BTreeSet<_>>();
    let role_conflicts = workspace
        .roles()
        .filter(|role| imported_role_names.contains(role.name().as_str()))
        .map(|role| format!("role name {:?} already exists", role.name().as_str()))
        .collect();
    Ok(ImportPreview {
        counts: ImportCounts {
            roles: roles.len(),
            agents: agents.len(),
            tasks: tasks.len(),
            handoffs: handoffs.len(),
            nodes: canvas.nodes.len(),
            groups: canvas.groups.len(),
            connections: canvas.connections.len(),
        },
        referenced_paths: referenced_paths.into_iter().collect(),
        unresolved_launchers,
        role_conflicts,
        warnings: Vec::new(),
    })
}

fn ensure_launchers_resolved(
    preview: &ImportPreview,
    workspace: &Workspace,
    mappings: &BTreeMap<String, crate::domain::CommandPresetId>,
) -> Result<(), PortableError> {
    let unresolved = preview
        .unresolved_launchers
        .iter()
        .filter(|id| !mappings.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    if unresolved.is_empty() {
        for launcher_id in &preview.unresolved_launchers {
            let preset_id = mappings
                .get(launcher_id)
                .expect("resolved launcher was checked above");
            if workspace.command_preset(*preset_id).is_none() {
                return Err(PortableError::MissingReference(format!(
                    "command preset {preset_id} for launcher {launcher_id}"
                )));
            }
        }
        Ok(())
    } else {
        Err(PortableError::UnresolvedLaunchers(unresolved))
    }
}

fn validate_template(document: &TemplateDocumentV1) -> Result<(), PortableError> {
    if document.format != TEMPLATE_FORMAT || document.version != DOCUMENT_VERSION {
        return Err(PortableError::UnsupportedVersion {
            found: document.version,
            supported: DOCUMENT_VERSION,
        });
    }
    validate_body(
        &document.template.roles,
        &document.template.agents,
        &document.template.tasks,
        &document.template.handoffs,
        &document.template.canvas,
    )
}

fn validate_archive(document: &WorkspaceArchiveV1) -> Result<(), PortableError> {
    if document.format != ARCHIVE_FORMAT || document.version != DOCUMENT_VERSION {
        return Err(PortableError::UnsupportedVersion {
            found: document.version,
            supported: DOCUMENT_VERSION,
        });
    }
    if document.archive.settings.name.is_empty() {
        return Err(PortableError::InvalidDocument(
            "archive workspace name is empty".to_owned(),
        ));
    }
    validate_body(
        &document.archive.roles,
        &document.archive.agents,
        &document.archive.tasks,
        &document.archive.handoffs,
        &document.archive.canvas,
    )
}

fn validate_body(
    roles: &[RoleV1],
    agents: &[AgentV1],
    tasks: &[TaskV1],
    handoffs: &[HandoffV1],
    canvas: &CanvasV1,
) -> Result<(), PortableError> {
    for (label, count) in [
        ("roles", roles.len()),
        ("agents", agents.len()),
        ("tasks", tasks.len()),
        ("handoffs", handoffs.len()),
        ("nodes", canvas.nodes.len()),
        ("groups", canvas.groups.len()),
        ("connections", canvas.connections.len()),
    ] {
        if count > MAX_ITEMS {
            return Err(PortableError::ItemLimit {
                label,
                limit: MAX_ITEMS,
            });
        }
    }
    let mut ids = BTreeSet::new();
    for id in roles
        .iter()
        .map(|entry| &entry.id)
        .chain(agents.iter().map(|entry| &entry.id))
        .chain(tasks.iter().map(|entry| &entry.id))
        .chain(handoffs.iter().map(|entry| &entry.id))
        .chain(canvas.nodes.iter().map(|entry| &entry.id))
        .chain(canvas.groups.iter().map(|entry| &entry.id))
        .chain(canvas.connections.iter().map(|entry| &entry.id))
    {
        validate_symbolic_id(id)?;
        if !ids.insert(id) {
            return Err(PortableError::InvalidDocument(format!(
                "duplicate symbolic id {id:?}"
            )));
        }
    }
    for node in &canvas.nodes {
        if node.target.is_some() == node.content.is_some() {
            return Err(PortableError::InvalidDocument(format!(
                "node {:?} must have exactly one content form",
                node.id
            )));
        }
        CanvasPoint::new(node.x, node.y)
            .map_err(|error| PortableError::InvalidDomain(error.to_string()))?;
        CanvasSize::new(node.width, node.height)
            .map_err(|error| PortableError::InvalidDomain(error.to_string()))?;
        if let Some(content) = &node.content {
            validate_content(content)?;
        }
    }
    Ok(())
}

fn validate_content(content: &CanvasContentV1) -> Result<(), PortableError> {
    match content {
        CanvasContentV1::Note { path, title } => {
            project_path(path)?;
            name(title)?;
        }
        CanvasContentV1::FileTree { root } | CanvasContentV1::Artifact { path: root } => {
            project_path(root)?;
        }
        CanvasContentV1::Diff { path, .. } => {
            project_path(path)?;
        }
        CanvasContentV1::Text { markdown } => {
            CanvasText::new(markdown.clone())
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?;
        }
        CanvasContentV1::Portal {
            kind,
            selector,
            frame_rate_limit,
            ..
        } => {
            PortalTarget::new((*kind).into(), selector.clone())
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?;
            PortalPresentation::new(true, *frame_rate_limit).ok_or_else(|| {
                PortableError::InvalidDomain(
                    "portal frame rate limit must be greater than zero".to_owned(),
                )
            })?;
        }
        CanvasContentV1::Shape { shape } => {
            StrokeWidth::new(shape.stroke_width)
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?;
        }
        CanvasContentV1::Arrow { arrow } => {
            StrokeWidth::new(arrow.stroke_width)
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?;
            if let Some(label) = &arrow.label {
                name(label)?;
            }
        }
        CanvasContentV1::Freehand { freehand } => {
            StrokeWidth::new(freehand.stroke_width)
                .map_err(|error| PortableError::InvalidDomain(error.to_string()))?;
            if freehand.points.len() > crate::domain::Freehand::MAX_POINTS {
                return Err(PortableError::ItemLimit {
                    label: "freehand points",
                    limit: crate::domain::Freehand::MAX_POINTS,
                });
            }
        }
    }
    Ok(())
}

fn scan_template(template: &TemplateBodyV1) -> Vec<SecretWarning> {
    let mut warnings = Vec::new();
    scan_body(
        &mut warnings,
        &template.roles,
        &template.agents,
        &template.tasks,
        &template.handoffs,
        &template.canvas,
        "template",
    );
    warnings
}

fn scan_archive(archive: &WorkspaceArchiveV1) -> Vec<SecretWarning> {
    let mut warnings = Vec::new();
    scan_string(
        &mut warnings,
        "archive.settings.name",
        &archive.archive.settings.name,
    );
    if let Some(icon) = &archive.archive.settings.icon {
        scan_string(&mut warnings, "archive.settings.icon", icon);
    }
    if let Some(instructions) = &archive.archive.settings.instructions {
        scan_string(&mut warnings, "archive.settings.instructions", instructions);
    }
    scan_body(
        &mut warnings,
        &archive.archive.roles,
        &archive.archive.agents,
        &archive.archive.tasks,
        &archive.archive.handoffs,
        &archive.archive.canvas,
        "archive",
    );
    warnings
}

fn scan_body(
    warnings: &mut Vec<SecretWarning>,
    roles: &[RoleV1],
    agents: &[AgentV1],
    tasks: &[TaskV1],
    handoffs: &[HandoffV1],
    canvas: &CanvasV1,
    prefix: &str,
) {
    for role in roles {
        scan_string(
            warnings,
            &format!("{prefix}.roles[{}].name", role.id),
            &role.name,
        );
        scan_string(
            warnings,
            &format!("{prefix}.roles[{}].instructions", role.id),
            &role.instructions,
        );
    }
    for agent in agents {
        scan_string(
            warnings,
            &format!("{prefix}.agents[{}].name", agent.id),
            &agent.name,
        );
    }
    for task in tasks {
        scan_string(
            warnings,
            &format!("{prefix}.tasks[{}].title", task.id),
            &task.title,
        );
        scan_string(
            warnings,
            &format!("{prefix}.tasks[{}].prompt", task.id),
            &task.prompt,
        );
    }
    for handoff in handoffs {
        if let HandoffPayloadV1::Question { content } = &handoff.payload {
            scan_string(
                warnings,
                &format!("{prefix}.handoffs[{}].payload.content", handoff.id),
                content,
            );
        }
    }
    for node in &canvas.nodes {
        if let Some(content) = &node.content {
            let field = format!("{prefix}.canvas.nodes[{}].content", node.id);
            match content {
                CanvasContentV1::Note { path, title } => {
                    scan_string(warnings, &format!("{field}.path"), path);
                    scan_string(warnings, &format!("{field}.title"), title);
                }
                CanvasContentV1::FileTree { root } => {
                    scan_string(warnings, &format!("{field}.root"), root);
                }
                CanvasContentV1::Artifact { path } | CanvasContentV1::Diff { path, .. } => {
                    scan_string(warnings, &format!("{field}.path"), path);
                }
                CanvasContentV1::Text { markdown } => {
                    scan_string(warnings, &format!("{field}.markdown"), markdown);
                }
                CanvasContentV1::Portal { selector, .. } => {
                    scan_string(warnings, &format!("{field}.selector"), selector);
                }
                CanvasContentV1::Arrow { arrow } => {
                    if let Some(label) = &arrow.label {
                        scan_string(warnings, &format!("{field}.label"), label);
                    }
                }
                CanvasContentV1::Shape { .. } | CanvasContentV1::Freehand { .. } => {}
            }
        }
    }
}

fn scan_string(warnings: &mut Vec<SecretWarning>, field: &str, value: &str) {
    let lower = value.to_ascii_lowercase();
    if lower.contains("-----begin ") {
        warnings.push(SecretWarning {
            field: field.to_owned(),
            pattern: "private-key",
        });
    }
    if has_credential_token(&lower, "bearer ", 16) {
        warnings.push(SecretWarning {
            field: field.to_owned(),
            pattern: "bearer-token",
        });
    }
    for (label, prefix) in [
        ("api-key-assignment", "api_key="),
        ("secret-assignment", "secret="),
        ("password-assignment", "password="),
        ("token-assignment", "token="),
    ] {
        if has_assignment(&lower, prefix, 8) {
            warnings.push(SecretWarning {
                field: field.to_owned(),
                pattern: label,
            });
        }
    }
    for (label, prefix, minimum) in [
        ("github-token", "ghp_", 20),
        ("github-token", "github_pat_", 20),
        ("slack-token", "xoxb-", 16),
        ("aws-access-key", "akia", 16),
        ("openai-key", "sk-", 16),
    ] {
        if has_credential_token(&lower, prefix, minimum) {
            warnings.push(SecretWarning {
                field: field.to_owned(),
                pattern: label,
            });
        }
    }
}

fn has_assignment(value: &str, prefix: &str, minimum: usize) -> bool {
    let mut offset = 0;
    while let Some(relative) = value[offset..].find(prefix) {
        let start = offset + relative;
        if token_boundary(value, start) {
            let candidate = value[start + prefix.len()..].trim_start();
            let length = candidate
                .chars()
                .take_while(|character| !character.is_whitespace())
                .count();
            if length >= minimum {
                return true;
            }
        }
        offset = start + prefix.len();
    }
    false
}

fn has_credential_token(value: &str, prefix: &str, minimum: usize) -> bool {
    let mut offset = 0;
    while let Some(relative) = value[offset..].find(prefix) {
        let start = offset + relative;
        if token_boundary(value, start) {
            let length = value[start + prefix.len()..]
                .chars()
                .take_while(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
                })
                .count();
            if length >= minimum {
                return true;
            }
        }
        offset = start + prefix.len();
    }
    false
}

fn token_boundary(value: &str, start: usize) -> bool {
    start == 0
        || !value.as_bytes()[start - 1].is_ascii_alphanumeric()
            && value.as_bytes()[start - 1] != b'_'
}

fn symbolic(kind: &str, id: u64) -> String {
    format!("{kind}-{id}")
}

fn validate_symbolic_id(id: &str) -> Result<(), PortableError> {
    if id.is_empty()
        || id.len() > MAX_ID_CHARS
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(PortableError::InvalidDocument(format!(
            "invalid symbolic id {id:?}"
        )));
    }
    Ok(())
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

struct IdAllocator {
    next: u64,
}

impl IdAllocator {
    fn new(ids: impl Iterator<Item = u64>) -> Self {
        Self {
            next: ids.max().unwrap_or(0).saturating_add(1),
        }
    }

    fn allocate(&mut self) -> u64 {
        let current = self.next;
        self.next = self.next.saturating_add(1);
        current
    }
}

impl From<ConnectionKind> for ConnectionKindV1 {
    fn from(kind: ConnectionKind) -> Self {
        match kind {
            ConnectionKind::Coordination => Self::Coordination,
            ConnectionKind::Assignment => Self::Assignment,
            ConnectionKind::Dependency => Self::Dependency,
            ConnectionKind::Handoff => Self::Handoff,
            ConnectionKind::Reference => Self::Reference,
        }
    }
}

impl From<ConnectionKindV1> for ConnectionKind {
    fn from(kind: ConnectionKindV1) -> Self {
        match kind {
            ConnectionKindV1::Coordination => Self::Coordination,
            ConnectionKindV1::Assignment => Self::Assignment,
            ConnectionKindV1::Dependency => Self::Dependency,
            ConnectionKindV1::Handoff => Self::Handoff,
            ConnectionKindV1::Reference => Self::Reference,
        }
    }
}

impl From<NodeTarget> for NodeTargetV1 {
    fn from(target: NodeTarget) -> Self {
        match target {
            NodeTarget::Agent(id) => Self::Agent {
                id: symbolic("agent", id.get()),
            },
            NodeTarget::Task(id) => Self::Task {
                id: symbolic("task", id.get()),
            },
            NodeTarget::Handoff(id) => Self::Handoff {
                id: symbolic("handoff", id.get()),
            },
        }
    }
}

impl From<DiffComparison> for DiffComparisonV1 {
    fn from(_: DiffComparison) -> Self {
        Self::WorkingTreeAgainstHead
    }
}

impl From<DiffComparisonV1> for DiffComparison {
    fn from(_: DiffComparisonV1) -> Self {
        Self::WorkingTreeAgainstHead
    }
}

impl From<ShapeKind> for ShapeKindV1 {
    fn from(kind: ShapeKind) -> Self {
        match kind {
            ShapeKind::Rectangle => Self::Rectangle,
            ShapeKind::Ellipse => Self::Ellipse,
        }
    }
}

impl From<ShapeKindV1> for ShapeKind {
    fn from(kind: ShapeKindV1) -> Self {
        match kind {
            ShapeKindV1::Rectangle => Self::Rectangle,
            ShapeKindV1::Ellipse => Self::Ellipse,
        }
    }
}

impl From<CanvasPoint> for PointV1 {
    fn from(point: CanvasPoint) -> Self {
        Self {
            x: point.x(),
            y: point.y(),
        }
    }
}

impl From<NormalizedPoint> for PointV1 {
    fn from(point: NormalizedPoint) -> Self {
        Self {
            x: point.x(),
            y: point.y(),
        }
    }
}

impl TryFrom<PointV1> for NormalizedPoint {
    type Error = crate::domain::DrawingValidationError;

    fn try_from(point: PointV1) -> Result<Self, Self::Error> {
        Self::new(point.x, point.y)
    }
}

#[derive(Debug)]
pub enum PortableError {
    InvalidJson(serde_json::Error),
    UnsupportedFormat(String),
    UnsupportedVersion { found: u32, supported: u32 },
    InvalidDocument(String),
    InvalidDomain(String),
    MissingReference(String),
    EmptySelection,
    SizeLimit { limit: usize },
    ItemLimit { label: &'static str, limit: usize },
    UnresolvedLaunchers(Vec<String>),
    SecretDetected(Vec<SecretWarning>),
}

impl Display for PortableError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(formatter, "invalid portable JSON: {error}"),
            Self::UnsupportedFormat(format) => {
                write!(formatter, "unsupported portable format {format:?}")
            }
            Self::UnsupportedVersion { found, supported } => write!(
                formatter,
                "unsupported portable version {found}; this build supports {supported}"
            ),
            Self::InvalidDocument(detail) => {
                write!(formatter, "invalid portable document: {detail}")
            }
            Self::InvalidDomain(detail) => write!(
                formatter,
                "portable document cannot create domain value: {detail}"
            ),
            Self::MissingReference(reference) => write!(
                formatter,
                "portable document references missing {reference}"
            ),
            Self::EmptySelection => formatter.write_str("select at least one canvas node"),
            Self::SizeLimit { limit } => write!(
                formatter,
                "portable document exceeds the {limit}-byte size limit"
            ),
            Self::ItemLimit { label, limit } => write!(
                formatter,
                "portable document exceeds the {limit}-item limit for {label}"
            ),
            Self::UnresolvedLaunchers(launchers) => write!(
                formatter,
                "map these custom launchers before importing: {}",
                launchers.join(", ")
            ),
            Self::SecretDetected(warnings) => write!(
                formatter,
                "export blocked because suspicious material was found in {}: {}",
                warnings
                    .iter()
                    .map(|warning| warning.field.as_str())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", "),
                warnings
                    .iter()
                    .map(|warning| warning.pattern)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl Error for PortableError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidJson(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::domain::{
        CanvasPoint, CanvasSize, DomainCommand, Floor, FloorLifecycle, Floors, TaskState,
        WorkspaceDirectory, WorkspaceId,
    };

    fn workspace() -> Workspace {
        Workspace::new(WorkspaceId::new(1), Name::new("Source").unwrap())
    }

    #[test]
    fn template_has_relative_geometry_and_symbolic_ids() {
        let mut workspace = workspace();
        workspace
            .execute(DomainCommand::AddRole(Role::new(
                RoleId::new(40),
                Name::new("Builder").unwrap(),
                Content::new("Build safely").unwrap(),
            )))
            .unwrap();
        workspace
            .execute(DomainCommand::AddAgent(Agent::new(
                AgentId::new(90),
                Name::new("Ada").unwrap(),
                Some(RoleId::new(40)),
            )))
            .unwrap();
        workspace
            .execute(DomainCommand::AddNode(Node::new(
                NodeId::new(700),
                NodeTarget::Agent(AgentId::new(90)),
                CanvasPoint::new(100.0, 200.0).unwrap(),
                CanvasSize::new(240.0, 160.0).unwrap(),
            )))
            .unwrap();

        let payload = export_template(&workspace, &[NodeId::new(700)]).unwrap();
        assert!(payload.contains("role-40"));
        assert!(payload.contains("agent-90"));
        assert!(payload.contains("\"x\": 0.0"));
        let document = decode_template(&payload).unwrap();
        assert_eq!(document.template.canvas.nodes[0].x, 0.0);
        assert_eq!(document.template.canvas.nodes[0].y, 0.0);
    }

    #[test]
    fn portal_template_round_trips_configuration_without_a_live_session() {
        let mut source = workspace();
        source
            .execute(DomainCommand::AddNode(Node::with_content(
                NodeId::new(9),
                CanvasNodeContent::Portal(PortalConfig::browser("https://example.test").unwrap()),
                CanvasPoint::new(12.0, 24.0).unwrap(),
                CanvasSize::new(320.0, 240.0).unwrap(),
            )))
            .unwrap();

        let document =
            decode_template(&export_template(&source, &[NodeId::new(9)]).unwrap()).unwrap();
        assert_eq!(document.version, DOCUMENT_VERSION);
        assert!(matches!(
            document.template.canvas.nodes[0].content.as_ref(),
            Some(CanvasContentV1::Portal { .. })
        ));

        let mut destination = workspace();
        let plan = import_template(
            &document,
            &destination,
            PointV1 { x: 0.0, y: 0.0 },
            &BTreeMap::new(),
        )
        .unwrap();
        for command in plan.commands {
            destination.execute(command).unwrap();
        }
        assert_eq!(
            destination.node(NodeId::new(1)).unwrap().content(),
            &CanvasNodeContent::Portal(PortalConfig::browser("https://example.test").unwrap())
        );
    }

    #[test]
    fn archive_rejects_secret_like_free_form_fields() {
        let mut workspace = workspace();
        workspace
            .execute(DomainCommand::UpdateWorkspaceSettings(
                crate::domain::WorkspaceSettings::new(
                    Name::new("Workspace").unwrap(),
                    None,
                    None,
                    Some(Content::new("password=do-not-export").unwrap()),
                ),
            ))
            .unwrap();
        let error = export_workspace_archive(&workspace).unwrap_err();
        assert!(matches!(error, PortableError::SecretDetected(_)));
    }

    #[test]
    fn template_rejects_secret_like_canvas_content() {
        let mut workspace = workspace();
        workspace
            .execute(DomainCommand::AddNode(Node::with_content(
                NodeId::new(1),
                CanvasNodeContent::Text {
                    markdown: CanvasText::new("password=synthetic-review-canary").unwrap(),
                },
                CanvasPoint::new(0.0, 0.0).unwrap(),
                CanvasSize::new(240.0, 160.0).unwrap(),
            )))
            .unwrap();

        let error = export_template(&workspace, &[NodeId::new(1)]).unwrap_err();
        assert!(matches!(error, PortableError::SecretDetected(_)));
    }

    #[test]
    fn secret_scanner_respects_token_boundaries_and_key_shapes() {
        let mut workspace = workspace();
        workspace
            .execute(DomainCommand::AddNode(Node::with_content(
                NodeId::new(1),
                CanvasNodeContent::Text {
                    markdown: CanvasText::new("Use task-based planning").unwrap(),
                },
                CanvasPoint::new(0.0, 0.0).unwrap(),
                CanvasSize::new(240.0, 160.0).unwrap(),
            )))
            .unwrap();

        assert!(export_template(&workspace, &[NodeId::new(1)]).is_ok());
        assert!(!has_credential_token("task-based planning", "sk-", 16));
        assert!(has_credential_token(
            "sk-synthetic-review-canary",
            "sk-",
            16
        ));
    }

    #[test]
    fn retry_relationships_round_trip_with_terminal_predecessor_state() {
        let mut source = workspace();
        source
            .execute(DomainCommand::AddTask(Task::new(
                TaskId::new(1),
                Name::new("First attempt").unwrap(),
                Content::new("Try the change").unwrap(),
                None,
                None,
            )))
            .unwrap();
        source
            .execute(DomainCommand::TransitionTask {
                task_id: TaskId::new(1),
                to: TaskState::Failed,
            })
            .unwrap();
        source
            .execute(DomainCommand::AddTask(Task::new(
                TaskId::new(2),
                Name::new("Second attempt").unwrap(),
                Content::new("Try the change again").unwrap(),
                None,
                Some(TaskId::new(1)),
            )))
            .unwrap();
        source
            .execute(DomainCommand::AddTask(Task::new(
                TaskId::new(3),
                Name::new("Cancelled attempt").unwrap(),
                Content::new("Cancel the change").unwrap(),
                None,
                None,
            )))
            .unwrap();
        source
            .execute(DomainCommand::TransitionTask {
                task_id: TaskId::new(3),
                to: TaskState::Cancelled,
            })
            .unwrap();
        source
            .execute(DomainCommand::AddTask(Task::new(
                TaskId::new(4),
                Name::new("Cancelled retry").unwrap(),
                Content::new("Try a different path").unwrap(),
                None,
                Some(TaskId::new(3)),
            )))
            .unwrap();

        let archive =
            decode_workspace_archive(&export_workspace_archive(&source).unwrap()).unwrap();
        assert_eq!(
            archive.archive.tasks[1].retry_source_state,
            Some(RetrySourceStateV1::Failed)
        );
        assert_eq!(
            archive.archive.tasks[3].retry_source_state,
            Some(RetrySourceStateV1::Cancelled)
        );

        let mut destination = workspace();
        let plan = import_workspace_archive(&archive, &destination, &BTreeMap::new()).unwrap();
        for command in plan.commands {
            destination.execute(command).unwrap();
        }
        assert_eq!(
            destination.task(TaskId::new(1)).unwrap().state(),
            TaskState::Failed
        );
        assert_eq!(
            destination.task(TaskId::new(2)).unwrap().retry_of(),
            Some(TaskId::new(1))
        );
        assert_eq!(
            destination.task(TaskId::new(3)).unwrap().state(),
            TaskState::Cancelled
        );
        assert_eq!(
            destination.task(TaskId::new(4)).unwrap().retry_of(),
            Some(TaskId::new(3))
        );
    }

    #[test]
    fn import_merges_active_floor_without_duplicating_hidden_nodes() {
        let mut source = workspace();
        source
            .execute(DomainCommand::AddNode(Node::with_content(
                NodeId::new(1),
                CanvasNodeContent::Text {
                    markdown: CanvasText::new("imported").unwrap(),
                },
                CanvasPoint::new(100.0, 100.0).unwrap(),
                CanvasSize::new(240.0, 160.0).unwrap(),
            )))
            .unwrap();
        let document =
            decode_template(&export_template(&source, &[NodeId::new(1)]).unwrap()).unwrap();

        let mut destination = workspace();
        destination
            .execute(DomainCommand::AddNode(Node::with_content(
                NodeId::new(10),
                CanvasNodeContent::Text {
                    markdown: CanvasText::new("hidden").unwrap(),
                },
                CanvasPoint::new(0.0, 0.0).unwrap(),
                CanvasSize::new(240.0, 160.0).unwrap(),
            )))
            .unwrap();
        let before = destination.floors().clone();
        let mut floors = Floors::default();
        for id in [1, 2] {
            floors.entries.insert(
                id,
                Floor {
                    name: Name::new(format!("floor-{id}")).unwrap(),
                    directory: WorkspaceDirectory::new(format!("/floor-{id}")).unwrap(),
                    repository: WorkspaceDirectory::new(format!("/floor-{id}")).unwrap(),
                    branch: None,
                    base_revision: "HEAD".to_owned(),
                    base_branch: None,
                    managed: false,
                    ownership_token: None,
                    owner: None,
                    dirty: false,
                    lifecycle: FloorLifecycle::Available,
                },
            );
        }
        floors.active = Some(1);
        destination
            .execute(DomainCommand::ReplaceFloors {
                before,
                after: floors,
            })
            .unwrap();

        let plan = import_template(
            &document,
            &destination,
            PointV1 { x: 0.0, y: 0.0 },
            &BTreeMap::new(),
        )
        .unwrap();
        for command in plan.commands {
            destination.execute(command).unwrap();
        }
        assert_eq!(destination.all_canvas_layout().nodes().len(), 2);
        assert_eq!(destination.canvas_layout().nodes().len(), 1);
        assert_eq!(destination.canvas_layout().nodes()[0].id(), NodeId::new(11));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let error = decode_template(r#"{"format":"openpodium-template","version":1,"extra":true,"template":{"origin":{"x":0.0,"y":0.0},"roles":[],"agents":[],"tasks":[],"handoffs":[],"canvas":{"nodes":[],"groups":[],"connections":[]}}}"#).unwrap_err();
        assert!(matches!(error, PortableError::InvalidJson(_)));
    }

    #[test]
    fn archive_omits_machine_paths_and_runtime_containers() {
        let mut workspace = workspace();
        workspace
            .execute(DomainCommand::UpdateWorkspaceSettings(
                crate::domain::WorkspaceSettings::new(
                    Name::new("Portable").unwrap(),
                    None,
                    Some(crate::domain::WorkspaceDirectory::new("/machine/project").unwrap()),
                    Some(Content::new("Keep the review focused").unwrap()),
                ),
            ))
            .unwrap();

        let payload = export_workspace_archive(&workspace).unwrap();
        assert!(!payload.contains("/machine/project"));
        assert!(!payload.contains("ownership_token"));
        assert!(!payload.contains("environment_profiles"));
        assert!(!payload.contains("attachments"));
        assert!(decode_workspace_archive(&payload).is_ok());
    }

    #[test]
    fn custom_launchers_are_explicitly_unresolved_in_preview() {
        let mut workspace = workspace();
        workspace
            .execute(DomainCommand::AddCommandPreset(
                crate::domain::CommandPreset::new(
                    crate::domain::CommandPresetId::new(8),
                    Name::new("Private launcher").unwrap(),
                    "tool",
                    Vec::new(),
                )
                .unwrap(),
            ))
            .unwrap();
        workspace
            .execute(DomainCommand::AddAgent(Agent::with_program(
                AgentId::new(4),
                Name::new("Custom").unwrap(),
                None,
                AgentProgram::Custom(crate::domain::CommandPresetId::new(8)),
            )))
            .unwrap();
        workspace
            .execute(DomainCommand::AddNode(Node::new(
                NodeId::new(5),
                NodeTarget::Agent(AgentId::new(4)),
                CanvasPoint::new(0.0, 0.0).unwrap(),
                CanvasSize::new(240.0, 160.0).unwrap(),
            )))
            .unwrap();

        let document =
            decode_template(&export_template(&workspace, &[NodeId::new(5)]).unwrap()).unwrap();
        let preview = preview_template_import(&document, &workspace).unwrap();
        assert_eq!(preview.unresolved_launchers, vec!["launcher-8"]);
        assert!(matches!(
            import_template(
                &document,
                &workspace,
                PointV1 { x: 0.0, y: 0.0 },
                &BTreeMap::new(),
            ),
            Err(PortableError::UnresolvedLaunchers(_))
        ));
    }

    #[test]
    fn version_zero_template_fixture_migrates_missing_origin() {
        let mut value = serde_json::json!({
            "format": TEMPLATE_FORMAT,
            "version": 1,
            "template": {
                "origin": {"x": 0.0, "y": 0.0},
                "roles": [],
                "agents": [],
                "tasks": [],
                "handoffs": [],
                "canvas": {"nodes": [], "groups": [], "connections": []}
            }
        });
        let object = value.as_object_mut().unwrap();
        object.insert("version".to_owned(), Value::from(0));
        object
            .get_mut("template")
            .and_then(Value::as_object_mut)
            .unwrap()
            .remove("origin");

        let migrated = decode_template(&value.to_string()).unwrap();
        assert_eq!(migrated.version, DOCUMENT_VERSION);
        assert_eq!(migrated.template.origin, PointV1 { x: 0.0, y: 0.0 });
    }

    #[test]
    fn template_import_remaps_groups_connections_and_relative_positions() {
        let mut source = workspace();
        let first = Node::with_content(
            NodeId::new(10),
            CanvasNodeContent::Text {
                markdown: CanvasText::new("First").unwrap(),
            },
            CanvasPoint::new(100.0, 200.0).unwrap(),
            CanvasSize::new(240.0, 160.0).unwrap(),
        );
        let second = Node::with_content(
            NodeId::new(11),
            CanvasNodeContent::Text {
                markdown: CanvasText::new("Second").unwrap(),
            },
            CanvasPoint::new(400.0, 200.0).unwrap(),
            CanvasSize::new(240.0, 160.0).unwrap(),
        );
        let layout = CanvasLayout::new(
            vec![first, second],
            vec![NodeGroup::new(
                NodeGroupId::new(20),
                [NodeId::new(10), NodeId::new(11)],
            )],
            vec![Connection::new(
                ConnectionId::new(30),
                NodeId::new(10),
                NodeId::new(11),
                ConnectionKind::Reference,
            )],
        );
        source
            .execute(DomainCommand::ReplaceCanvas {
                before: CanvasLayout::default(),
                after: layout,
            })
            .unwrap();

        let document =
            decode_template(&export_template(&source, &[NodeId::new(10)]).unwrap()).unwrap();
        let mut destination = workspace();
        let plan = import_template(
            &document,
            &destination,
            PointV1 { x: 500.0, y: 700.0 },
            &BTreeMap::new(),
        )
        .unwrap();
        for command in plan.commands {
            destination.execute(command).unwrap();
        }

        let imported = destination.all_canvas_layout();
        assert_eq!(imported.nodes().len(), 2);
        assert_eq!(imported.groups().len(), 1);
        assert_eq!(imported.connections().len(), 1);
        assert_eq!(imported.nodes()[0].id(), NodeId::new(1));
        assert_eq!(imported.nodes()[0].position().x(), 500.0);
        assert_eq!(imported.nodes()[1].id(), NodeId::new(2));
        assert_eq!(imported.connections()[0].source(), NodeId::new(1));
        assert_eq!(imported.connections()[0].target(), NodeId::new(2));
    }

    proptest! {
        #[test]
        fn arbitrary_portable_documents_are_rejected_or_fully_validated(input in any::<String>()) {
            let _ = decode_template(&input);
            let _ = decode_workspace_archive(&input);
        }
    }
}
