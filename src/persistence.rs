//! Durable SQLite event journal and workspace recovery.

mod canvas;
mod codec;
mod error;
mod roles;
mod storage;

pub use canvas::{CanvasTransferError, export_canvas_fragment, import_canvas_fragment};
pub use error::PersistenceError;
pub use roles::{RoleTransferError, export_role, import_role};
pub use storage::Journal;

#[cfg(test)]
mod tests;
