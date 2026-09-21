//! The durable routine scheduler.
//!
//! The scheduler sits above handoff orchestration. It owns dependency
//! readiness, approvals, output binding, resource reservations, and
//! interruption handling; the task lifecycle and handoff delivery stay where
//! they already were.
//!
//! Every decision it makes is recorded before the work it describes is
//! dispatched, so restarting OpenPodium reproduces the same orchestration from
//! the journal. Agent responses still vary; the scheduling does not.

mod engine;
pub use engine::*;

mod triggers;
pub use triggers::*;

#[cfg(test)]
mod tests;
