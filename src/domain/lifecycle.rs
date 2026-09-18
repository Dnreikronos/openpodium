use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Starting,
    Running,
    Waiting,
    Completed,
    Failed,
    Stopped,
}

impl AgentState {
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Starting, Self::Running | Self::Failed | Self::Stopped)
                | (
                    Self::Running,
                    Self::Waiting | Self::Completed | Self::Failed | Self::Stopped
                )
                | (
                    Self::Waiting,
                    Self::Running | Self::Completed | Self::Failed | Self::Stopped
                )
        )
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

impl Display for AgentState {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Queued,
    Delivered,
    Running,
    Blocked,
    Completed,
    Failed,
    Cancelled,
}

impl TaskState {
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Queued, Self::Delivered | Self::Cancelled)
                | (
                    Self::Delivered,
                    Self::Running | Self::Failed | Self::Cancelled
                )
                | (
                    Self::Running,
                    Self::Blocked | Self::Completed | Self::Failed | Self::Cancelled
                )
                | (
                    Self::Blocked,
                    Self::Running | Self::Failed | Self::Cancelled
                )
        )
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Delivered => "delivered",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl Display for TaskState {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
