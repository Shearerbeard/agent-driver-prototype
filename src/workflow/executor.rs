//! Deterministic workflow executor.
//!
//! Applies one step at a time in declaration (topological) order via
//! `SidecarClient::call_tool`. On step failure it runs declared rollbacks
//! of completed steps in reverse completion order. A failed rollback
//! stops the unwind and reports both failures. Cancellation mid-apply
//! halts dispatch and records residual applied steps without unwinding.

use std::collections::BTreeMap;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::mcp_client::{SidecarClient, SidecarError, SidecarToolArgs};
use crate::workflow::plan::{
    ExportName, RollbackSpec, StepId, ValidatedWorkflowSpec, WorkflowSpec, WorkflowStep,
};
use crate::workflow::resolve::{
    ExportEnvironment, ResolveError, capture_exports, parse_tool_result, resolve_arguments,
};

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
/// checked before each dispatch: a cancelled run halts without unwinding
/// and reports the steps already applied as its residual state. A failed
/// step unwinds the completed steps in reverse completion order; a
/// rollback that itself fails stops the unwind, and both failures plus
/// the steps still applied come back in the run record. Every terminal
/// state — success, failure, cancellation — is an `Ok` run record, the
/// tool observation the coordinator replans from.
pub async fn execute_workflow(
    spec: &ValidatedWorkflowSpec,
    client: &SidecarClient,
    cancel: &CancellationToken,
) -> Result<RunRecord, ExecuteError> {
    let spec = spec.spec();
    let steps = &spec.steps[..];
    // Every step starts NotStarted; only a dispatched step's record ever
    // changes, so the unstarted records hold this status by construction.
    let mut records: Vec<StepRecord> = steps
        .iter()
        .map(|step| StepRecord {
            id: step.id.clone(),
            status: StepStatus::NotStarted,
            result: None,
            rollback: None,
        })
        .collect();
    let mut env = ExportEnvironment::new();
    // Applied step positions in completion order. Apply is sequential, so
    // this coincides with declaration order restricted to the applied
    // steps, but the unwind's rule is pinned as reverse *completion*
    // order, so the run records the completion sequence explicitly
    // rather than leaning on the coincidence.
    let mut completed: Vec<usize> = Vec::new();

    for (position, step) in steps.iter().enumerate() {
        // Between dispatches: a cancellation here halts the run without
        // unwinding. The steps never dispatched hold NotStarted; the
        // applied ones are the residual state the coordinator replans
        // against.
        if cancel.is_cancelled() {
            return Ok(cancelled_record(spec, records, steps, &completed));
        }
        match apply_step(step, &env, client).await {
            Ok((result, exports)) => {
                // Completed steps' exports stay in the environment for the
                // rest of the run — later steps and every rollback resolve
                // against them; the unwind does not prune.
                env.insert(step.id.clone(), exports);
                completed.push(position);
                records[position] = StepRecord {
                    id: step.id.clone(),
                    status: StepStatus::Applied,
                    result: Some(result),
                    rollback: None,
                };
            }
            Err(error) => {
                records[position].status = StepStatus::Failed;
                // D11: a cancellation observed while the call was in
                // flight still interrupts the run. The failed step is
                // recorded truthfully, but no rollback dispatches after
                // the operator interrupted the apply — halt, record
                // residual, do not unwind.
                if cancel.is_cancelled() {
                    return Ok(cancelled_record(spec, records, steps, &completed));
                }
                let (rollback_failure, residual) =
                    unwind_completed_steps(steps, &mut records, &completed, &env, client).await;
                return Ok(RunRecord {
                    goal: spec.goal.clone(),
                    steps: records,
                    outcome: RunOutcome::Failed {
                        step: step.id.clone(),
                        error,
                        rollback_failure,
                        residual,
                    },
                });
            }
        }
    }

    // A cancellation that landed during the final in-flight call
    // interrupts the run too: every step applied, none unwound, the
    // outcome records the interrupt rather than a clean completion.
    if cancel.is_cancelled() {
        return Ok(cancelled_record(spec, records, steps, &completed));
    }

    Ok(RunRecord {
        goal: spec.goal.clone(),
        steps: records,
        outcome: RunOutcome::Complete,
    })
}

/// The D11 cancellation record: applied steps are the residual set the
/// coordinator replans against, and nothing unwinds.
fn cancelled_record(
    spec: &WorkflowSpec,
    records: Vec<StepRecord>,
    steps: &[WorkflowStep],
    completed: &[usize],
) -> RunRecord {
    RunRecord {
        goal: spec.goal.clone(),
        steps: records,
        outcome: RunOutcome::Cancelled {
            residual: completed
                .iter()
                .map(|&applied| steps[applied].id.clone())
                .collect(),
        },
    }
}

/// Apply one step: resolve its arguments against the exports of the
/// completed steps, call its tool, parse the result text as JSON, and
/// capture its declared exports.
///
/// Every failure on this path — a missing `$from` target, a bounds
/// violation, a non-JSON result, an export path that lifts nothing, or
/// the tool-call round trip itself — is the step's failure.
async fn apply_step(
    step: &WorkflowStep,
    env: &ExportEnvironment,
    client: &SidecarClient,
) -> Result<(Value, BTreeMap<ExportName, Value>), ExecuteError> {
    let args = resolve_arguments(&step.args, env, &step.id)?;
    let args = SidecarToolArgs::from_value(args)?;
    let content = client.call_tool(&step.tool, &args).await?;
    let result = parse_tool_result(&content)?;
    let exports = capture_exports(&step.id, &step.exports, &result)?;
    Ok((result, exports))
}

/// Run one completed step's declared rollback.
///
/// The compensating arguments resolve against the same export environment
/// as the apply path — it still holds every completed step's exports, the
/// owning step's own included. The rollback's result text is not parsed:
/// nothing consumes it, and a compensating call succeeded when its round
/// trip reports success.
async fn run_rollback(
    owner: &StepId,
    rollback: &RollbackSpec,
    env: &ExportEnvironment,
    client: &SidecarClient,
) -> Result<(), ExecuteError> {
    let args = resolve_arguments(&rollback.args, env, owner)?;
    let args = SidecarToolArgs::from_value(args)?;
    client.call_tool(&rollback.tool, &args).await?;
    Ok(())
}

/// Unwind the completed steps in reverse completion order after a step
/// failure, marking each step's record as the unwind passes it.
///
/// A completed step without a declared rollback is read-only: there is
/// nothing to undo, so nothing of it remains in effect and the unwind
/// records it [`StepStatus::Unwound`] with `rollback: None` — no attempt
/// was made.
///
/// A rollback that itself fails stops the unwind: that step records
/// [`StepStatus::RollbackFailed`] with the attempt's outcome, every
/// completed step the stopped unwind never reached records
/// [`StepStatus::NotUnwound`], and the failure comes back as the run's
/// `rollback_failure` beside the step failure that started the unwind.
/// All rollback failures — a failed compensating tool call as much as a
/// resolution failure in the rollback's own arguments — surface as the
/// one loud [`ExecuteError::RollbackFailed`] shape.
///
/// Returns the first failed rollback, if any, and the residual set: the
/// completed steps whose rollbacks never ran
/// ([`StepStatus::NotUnwound`] only, in completion order). A
/// [`StepStatus::RollbackFailed`] step is still applied, but its rollback
/// did run and failed; it reports through the run's `rollback_failure`
/// and its own step record instead of doubling into the residual set.
async fn unwind_completed_steps(
    steps: &[WorkflowStep],
    records: &mut [StepRecord],
    completed: &[usize],
    env: &ExportEnvironment,
    client: &SidecarClient,
) -> (Option<(StepId, ExecuteError)>, Vec<StepId>) {
    let mut rollback_failure = None;
    for &position in completed.iter().rev() {
        let step = &steps[position];
        let Some(rollback) = &step.rollback else {
            records[position].status = StepStatus::Unwound;
            continue;
        };
        match run_rollback(&step.id, rollback, env, client).await {
            Ok(()) => {
                records[position].status = StepStatus::Unwound;
                records[position].rollback = Some(RollbackOutcome::Success);
            }
            Err(error) => {
                let message = error.to_string();
                records[position].status = StepStatus::RollbackFailed;
                records[position].rollback = Some(RollbackOutcome::Failed {
                    error: message.clone(),
                });
                rollback_failure = Some((
                    step.id.clone(),
                    ExecuteError::RollbackFailed {
                        step: step.id.clone(),
                        message,
                    },
                ));
                break;
            }
        }
    }
    // Whatever the unwind never reached still holds the Applied status it
    // carried before the unwind started: those steps record NotUnwound and
    // form the residual set — the applied steps whose rollbacks never ran
    // (the plan's wording). A RollbackFailed step is still applied too, but
    // its rollback DID run and failed; it is reported as the run's
    // rollback_failure and in its own step record, so it does not double
    // into the residual set the coordinator replans against.
    let mut residual = Vec::new();
    for &position in completed {
        if records[position].status == StepStatus::Applied {
            records[position].status = StepStatus::NotUnwound;
        }
        if records[position].status == StepStatus::NotUnwound {
            residual.push(steps[position].id.clone());
        }
    }
    (rollback_failure, residual)
}
