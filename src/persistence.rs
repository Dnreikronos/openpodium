//! Durable SQLite event journal and workspace recovery.

mod canvas;
mod codec;
mod error;
mod portable;
mod roles;
mod storage;

pub use canvas::{CanvasTransferError, export_canvas_fragment, import_canvas_fragment};
pub use error::PersistenceError;
pub use portable::{
    AgentProgramV1, ArrowV1, CanvasContentV1, CanvasNodeV1, CanvasV1, ConnectionKindV1,
    ConnectionV1, DiffComparisonV1, FreehandV1, HandoffPayloadV1, HandoffV1, ImportCounts,
    ImportPreview, NodeGroupV1, NodeTargetV1, PointV1, PortableError, PortableImport,
    PortableSettingsV1, RoleV1, SecretWarning, ShapeKindV1, ShapeV1, TaskV1, TemplateBodyV1,
    TemplateDocumentV1, WorkspaceArchiveBodyV1, WorkspaceArchiveV1, decode_template,
    decode_workspace_archive, export_template, export_workspace_archive, import_template,
    import_workspace_archive, preview_template_import, preview_workspace_archive_import,
};
pub use roles::{RoleTransferError, export_role, import_role};
pub use storage::Journal;

#[cfg(test)]
mod tests;
