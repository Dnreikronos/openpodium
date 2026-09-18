//! Durable SQLite event journal and workspace recovery.

mod codec;
mod error;
mod storage;

pub use error::PersistenceError;
pub use storage::Journal;

#[cfg(test)]
mod tests;
