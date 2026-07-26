//! Rejections produced by the loop's parsing constructors and its run driver.

use agent_driver_rs::{AgentLoopError, ConfigError};

use crate::context::ContextError;

/// A value one of this module's parsing constructors refused.
///
/// Variants name the rule that was violated; the caller still holds the
/// offending input, so no variant repeats it except where the underlying
/// producer supplies its own message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoordinatorLoopError {
    /// A zero-turn budget would stop the loop before the coordinator's first
    /// tool call, so the run could never produce an answer.
    #[error("loop budget must allow at least one turn")]
    ZeroTurnBudget,
    /// An empty final answer is indistinguishable from no answer, which the
    /// outcome already represents as its own case.
    #[error("final response text is empty")]
    EmptyFinalResponse,
    /// Plan ids are derived from plan arguments, never authored, so a value
    /// outside the derived alphabet names no plan that could exist.
    #[error("plan id is not a lowercase hex digest of the derived width")]
    MalformedPlanId,
    /// The step tree the model sent does not flatten into a task list.
    #[error("plan steps are not executable: {0}")]
    UnexecutableSteps(String),
    /// A worker submission carried no attested summary, result, or usable
    /// evidence body.
    #[error("worker submission is not usable evidence: {0}")]
    UnusableSubmission(#[from] ContextError),
}

/// A coordinator run that ended before it could reach an outcome.
///
/// Distinct from [`CoordinatorLoopError`]: these are failures of the run
/// itself, not rejections of a value. Stop reasons the substrate reports
/// gracefully (tool errors, cancellation, provider stream failure) are
/// outcomes, not errors, and travel in
/// [`CoordinatorOutcome`](super::CoordinatorOutcome) instead.
#[derive(Debug, thiserror::Error)]
pub enum CoordinatorRunError {
    /// The coordinator session could not be built from the supplied
    /// provider and model.
    #[error(transparent)]
    Session(#[from] ConfigError),
    /// The substrate loop failed outright instead of returning a stop
    /// reason.
    #[error(transparent)]
    AgentLoop(#[from] AgentLoopError),
}

/// A terminal slot already holds the value that won.
///
/// Returned to the second and later writers so the rejection reaches the
/// model as an observation rather than silently overwriting the answer the
/// run already committed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("this run already recorded its answer; the first one stands")]
pub struct AlreadyRecorded;
