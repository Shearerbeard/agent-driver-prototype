//! Deterministic workflow executor.
//!
//! Applies one step at a time in declaration (topological) order via
//! `SidecarClient::call_tool`. On step failure it runs declared rollbacks
//! of completed steps in reverse completion order. A failed rollback
//! stops the unwind and reports both failures. Cancellation mid-apply
//! halts dispatch and records residual applied steps without unwinding.

#![expect(unused_variables)]
#![allow(clippy::unused_async)]

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::mcp_client::{SidecarClient, SidecarError};
use crate::workflow::plan::{StepId, ValidatedWorkflowSpec};
use crate::workflow::resolve::ResolveError;

/// The status of one step in the run record vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepStatus {
    /// The step has not been reached yet.
    NotStarted,
    /// The step applied successfully.
    Applied,
    /// The step failed to apply.
    Failed,
    /// The step applied and its rollback completed successfully.
    Unwound,
    /// The step applied and its rollback failed.
    RollbackFailed,
    /// The step applied but was not unwound because the unwind stopped
    /// earlier.
    NotUnwound,
}

/// The outcome of one rollback attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RollbackOutcome {
    /// The rollback tool call succeeded.
    Success,
    /// The rollback tool call failed with a message.
    Failed { error: String },
}

/// A per-step observation in the run record.
#[derive(Clone, Debug, PartialEq)]
pub struct StepRecord {
    /// The step's id.
    pub id: StepId,
    /// The step's terminal status in this run.
    pub status: StepStatus,
    /// The parsed JSON result, when the step applied successfully.
    pub result: Option<Value>,
    /// The rollback attempt, when one was made.
    pub rollback: Option<RollbackOutcome>,
}

/// The overall result of executing a workflow.
#[derive(Clone, Debug, PartialEq)]
pub struct RunRecord {
    /// The workflow's goal.
    pub goal: String,
    /// Per-step observations in declaration order.
    pub steps: Vec<StepRecord>,
    /// The terminal state of the run.
    pub outcome: RunOutcome,
}

/// The terminal state of a workflow run.
#[derive(Clone, Debug, PartialEq)]
pub enum RunOutcome {
    /// Every step applied; no rollback was necessary.
    Complete,
    /// A step failed. Completed steps were unwound in reverse order; the
    /// residual set lists steps still applied after the unwind stopped.
    Failed {
        /// The step that failed to apply.
        step: StepId,
        /// Why it failed.
        error: ExecuteError,
        /// The first rollback attempt that failed, if the unwind stopped
        /// because of a rollback failure.
        rollback_failure: Option<(StepId, ExecuteError)>,
        /// Steps still applied after the run stopped.
        residual: Vec<StepId>,
    },
    /// The run was cancelled before every step applied.
    Cancelled {
        /// Steps that had already been applied.
        residual: Vec<StepId>,
    },
}

/// Why execution stopped short of success.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ExecuteError {
    /// Resolving references or checking bounds for a step failed.
    #[error("resolution failed: {0}")]
    Resolve(#[from] ResolveError),
    /// A tool call round trip failed.
    #[error("tool call failed: {0}")]
    Sidecar(#[from] SidecarError),
    /// A step's tool call returned an error.
    #[error("step '{step}' failed: {message}")]
    StepFailed { step: StepId, message: String },
    /// A step's rollback tool call returned an error.
    #[error("rollback of step '{step}' failed: {message}")]
    RollbackFailed { step: StepId, message: String },
}

/// Execute a validated workflow against the sidecar.
///
/// Steps apply one at a time in declaration order. Cancellation is
/// checked before each dispatch.
pub async fn execute_workflow(
    spec: &ValidatedWorkflowSpec,
    client: &SidecarClient,
    cancel: &CancellationToken,
) -> Result<RunRecord, ExecuteError> {
    todo!()
}
