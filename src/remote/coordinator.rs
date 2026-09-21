use super::protocol::{CommandOutcome, CommandReceipt, DeviceId, SteeringAction, SteeringCommand};
use super::session::CommandDisposition;
use super::storage::{RemoteState, RemoteStateStore, RemoteStorageError};

/// Application boundary for the four operations a paired device may request.
///
/// Implementations must route approval through the existing local policy; a
/// remote approval is an answer to a pending prompt, not a policy bypass.
pub trait SteeringHandler {
    fn prompt(&mut self, workspace_id: u64, agent_id: u64, body: &str) -> Result<(), String>;
    fn approve(&mut self, workspace_id: u64, approval_id: &str) -> Result<(), String>;
    fn cancel(&mut self, workspace_id: u64, task_id: u64) -> Result<(), String>;
    fn resume(&mut self, workspace_id: u64, task_id: u64) -> Result<(), String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandHandling {
    Executed(CommandReceipt),
    Pending,
    Replayed(CommandReceipt),
}

/// Authorizes, durably deduplicates, dispatches, and records one command.
///
/// A recovered `Pending` command is deliberately not dispatched again because
/// its side effect may have completed before the prior process stopped.
pub fn handle_command(
    store: &mut RemoteStateStore,
    state: &mut RemoteState,
    device_id: DeviceId,
    command: &SteeringCommand,
    handler: &mut impl SteeringHandler,
) -> Result<CommandHandling, RemoteStorageError> {
    match store.begin_command(state, device_id, command)? {
        CommandDisposition::Pending => Ok(CommandHandling::Pending),
        CommandDisposition::Completed(receipt) => Ok(CommandHandling::Replayed(receipt)),
        CommandDisposition::Execute => {
            let result = match &command.action {
                SteeringAction::Prompt { agent_id, body } => {
                    handler.prompt(command.workspace_id, *agent_id, body)
                }
                SteeringAction::Approve { approval_id } => {
                    handler.approve(command.workspace_id, approval_id)
                }
                SteeringAction::Cancel { task_id } => {
                    handler.cancel(command.workspace_id, *task_id)
                }
                SteeringAction::Resume { task_id } => {
                    handler.resume(command.workspace_id, *task_id)
                }
            };
            let outcome = match result {
                Ok(()) => CommandOutcome::Applied,
                Err(detail) => CommandOutcome::Failed(detail),
            };
            let receipt = store.complete_command(state, command.id, outcome)?;
            Ok(CommandHandling::Executed(receipt))
        }
    }
}
