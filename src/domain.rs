//! Dependency-free durable domain types and transition rules.
//!
//! Entity identifiers are deliberately incompatible:
//!
//! ```compile_fail
//! use openpodium::domain::{AgentId, TaskId};
//!
//! fn find_agent(_id: AgentId) {}
//!
//! find_agent(TaskId::new(1));
//! ```

mod ids;
pub use ids::*;

mod value_objects;
pub use value_objects::*;

mod lifecycle;
pub use lifecycle::*;

mod environments;
pub use environments::*;

mod agents;
pub use agents::*;

mod chat;
pub use chat::*;

mod orchestration;
pub use orchestration::*;

mod entities;
pub use entities::*;

mod canvas_content;
pub use canvas_content::*;

mod events;
pub use events::*;

mod workspace;
pub use workspace::*;

#[cfg(test)]
mod tests;
