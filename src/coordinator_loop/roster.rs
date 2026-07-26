//! The set of worker names a plan may assign work to.

use crate::config::OrchestrationConfig;
use crate::context::WorkerRole;

use super::error::CoordinatorLoopError;

/// The workers this run has configured, in configuration order.
///
/// The roster is what makes worker assignment checkable: the planning schema
/// offers exactly these names and the plan parse rejects anything else, so a
/// plan can never be dispatched to a worker that does not exist. An empty
/// roster is a run with no workers at all, where naming any worker is the
/// error.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkerRoster(Vec<WorkerRole>);

impl WorkerRoster {
    /// Read the roster from an orchestration configuration.
    pub fn from_config(config: &OrchestrationConfig) -> Self {
        Self(
            config
                .available_worker_names()
                .into_iter()
                .filter_map(|name| WorkerRole::new(name).ok())
                .collect(),
        )
    }

    /// A run with no workers configured.
    pub fn empty() -> Self {
        Self::default()
    }

    /// The configured roles, in configuration order.
    pub fn names(&self) -> &[WorkerRole] {
        &self.0
    }

    /// Whether the roster is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Check a worker name a plan proposed.
    ///
    /// # Errors
    ///
    /// Returns [`CoordinatorLoopError::UnknownWorker`] when no configured
    /// worker carries the name.
    pub fn check(&self, name: &str) -> Result<(), CoordinatorLoopError> {
        if self.0.iter().any(|role| role.as_str() == name) {
            return Ok(());
        }
        Err(CoordinatorLoopError::UnknownWorker {
            name: name.to_owned(),
            available: self.listed(),
        })
    }

    /// The configured names as a comma-separated list, or `none` when no
    /// workers are configured.
    pub fn listed(&self) -> String {
        if self.0.is_empty() {
            return "none".to_owned();
        }
        self.0
            .iter()
            .map(WorkerRole::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    }
}
