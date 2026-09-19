//! Durable SQLite event journal and workspace recovery.

mod codec;
mod error;
mod roles;
mod storage;

pub use error::PersistenceError;
pub use roles::{RoleTransferError, export_role, import_role};
pub use storage::Journal;

#[cfg(test)]
mod tests;
