//! Durable typed handoff routing between authenticated IPC and agent terminals.

mod engine;
mod prompt;

pub use engine::{DeliveryRequest, OrchestrationError, Orchestrator};

#[cfg(test)]
mod tests;
