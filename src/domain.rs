#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceSummary {
    name: String,
    agent_count: usize,
}

impl WorkspaceSummary {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn agent_count(&self) -> usize {
        self.agent_count
    }
}

impl Default for WorkspaceSummary {
    fn default() -> Self {
        Self {
            name: "Welcome".to_owned(),
            agent_count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WorkspaceSummary;

    #[test]
    fn default_workspace_is_empty() {
        let workspace = WorkspaceSummary::default();

        assert_eq!(workspace.name(), "Welcome");
        assert_eq!(workspace.agent_count(), 0);
    }
}
