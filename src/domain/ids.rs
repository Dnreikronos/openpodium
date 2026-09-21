use std::fmt::{self, Display, Formatter};

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

define_id!(WorkspaceId);
define_id!(EnvironmentProfileId);
define_id!(CommandPresetId);
define_id!(RoleId);
define_id!(AgentId);
define_id!(ChatThreadId);
define_id!(ChatMessageId);
define_id!(ChatAttachmentId);
define_id!(TaskId);
define_id!(HandoffId);
define_id!(NodeId);
define_id!(NodeGroupId);
define_id!(ConnectionId);
define_id!(TimelineEventId);
define_id!(RoutineId);
define_id!(RoutineVersionId);
define_id!(RoutineStepId);
define_id!(RoutineTriggerId);
define_id!(RoutineRunId);
define_id!(RoutineAttemptId);
