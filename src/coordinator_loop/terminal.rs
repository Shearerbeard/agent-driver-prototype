//! The answer a run commits to, and the first-write-wins slot that holds it.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex, PoisonError};

use crate::context::{
    BlockedEntry, CompletedEntry, EvidenceText, FailedEntry, FailureReport, WorkerClaim,
};

use super::error::{AlreadyRecorded, CoordinatorLoopError};
use super::observation::{ExecutionObservation, TaskObservation};
use super::run_store::RunStore;

/// Opening line of the answer the host writes when the coordinator wrote
/// none but work had already run.
const FALLBACK_WITH_EVIDENCE: &str =
    "The run ended before the coordinator wrote an answer. These are the task results it had:";

/// The final answer the coordinator authored for the user.
///
/// Held privately so an empty answer cannot exist: a run that produced
/// nothing is reported as its own outcome, never as a blank response. The
/// summary normalises to `None` when blank, so "present but empty" and
/// "absent" are one state rather than two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalResponse {
    response: String,
    response_summary: Option<String>,
}

impl FinalResponse {
    /// Parse a final answer.
    ///
    /// # Errors
    ///
    /// Returns [`CoordinatorLoopError::EmptyFinalResponse`] when `response`
    /// is empty or whitespace-only.
    pub fn new(
        response: &str,
        response_summary: Option<&str>,
    ) -> Result<Self, CoordinatorLoopError> {
        if response.trim().is_empty() {
            return Err(CoordinatorLoopError::EmptyFinalResponse);
        }
        Ok(Self {
            response: response.to_owned(),
            response_summary: response_summary
                .filter(|summary| !summary.trim().is_empty())
                .map(str::to_owned),
        })
    }

    /// The answer text delivered to the user.
    pub fn response(&self) -> &str {
        &self.response
    }

    /// The model's own short-form gloss of the answer, when it wrote one.
    pub fn response_summary(&self) -> Option<&str> {
        self.response_summary.as_deref()
    }

    /// Write the answer the coordinator did not.
    ///
    /// Rendered from the most recent execution observation, which is the
    /// freshest evidence the run holds, so the user gets the work already
    /// paid for. When nothing was executed there are no results to salvage,
    /// and the answer instead states that the run ended before any plan ran
    /// and lists the plans created but never executed.
    ///
    /// Lives here rather than beside the outcome it serves, because it is
    /// the one caller that mints a response without a model behind it and
    /// so needs the private fields.
    pub fn host_fallback(runs: &RunStore) -> Self {
        let response = match runs.latest_execution() {
            Some(execution) => render_evidence_fallback(&execution),
            None => render_no_execution_fallback(runs),
        };
        Self {
            response,
            response_summary: None,
        }
    }
}

/// Render the freshest execution's per-task evidence, reusing the entry
/// renderers the continuation frame already owns.
fn render_evidence_fallback(execution: &ExecutionObservation) -> String {
    let mut text = String::from(FALLBACK_WITH_EVIDENCE);
    for task in execution.tasks() {
        text.push_str("\n\n");
        text.push_str(render_task(task).as_str());
    }
    text
}

/// One task's rendered entry.
fn render_task(task: &TaskObservation) -> String {
    match task {
        TaskObservation::Completed {
            label,
            evidence,
            artifacts,
        } => String::from(
            CompletedEntry {
                label: label.clone(),
                evidence: evidence.clone(),
                artifacts: artifacts.clone(),
            }
            .render(),
        ),
        TaskObservation::Failed {
            label,
            category,
            error,
            ..
        } => String::from(
            FailedEntry {
                label: label.clone(),
                report: FailureReport::Hard {
                    category: *category,
                    error: error.clone(),
                },
            }
            .render(),
        ),
        TaskObservation::Blocked { label } => String::from(
            BlockedEntry {
                label: label.clone(),
            }
            .render(),
        ),
    }
}

/// Render the fixed statement for a run that ended before any plan ran.
fn render_no_execution_fallback(runs: &RunStore) -> String {
    let ids = runs.plan_ids();
    let listed = if ids.is_empty() {
        "none".to_owned()
    } else {
        ids.iter()
            .map(|id| id.as_str().to_owned())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut text =
        String::from("The run ended before any plan was executed. No task results are available.");
    let _ = write!(text, " Plans created but not executed: {listed}.");
    text
}

/// A worker's attested result: what it claims it produced, plus the evidence
/// body backing the claim.
///
/// Both halves are parsed on the way in, so a submission that reaches the
/// slot is already usable as coordinator-visible evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerSubmission {
    claim: WorkerClaim,
    result: EvidenceText,
}

impl WorkerSubmission {
    /// Assemble an attested worker result.
    pub fn new(claim: WorkerClaim, result: EvidenceText) -> Self {
        Self { claim, result }
    }

    /// The worker's own summary and confidence.
    pub fn claim(&self) -> &WorkerClaim {
        &self.claim
    }

    /// The result body the claim is made about.
    pub fn result(&self) -> &EvidenceText {
        &self.result
    }
}

/// A shared slot that keeps the first value written to it.
///
/// The loop has no terminal-tool mechanism: a tool that "ends" the run
/// records its payload here and returns an ordinary acknowledgement, and the
/// run ends when the model stops calling tools or the budget fires. The slot
/// is what makes a second answer a rejected observation instead of a silent
/// overwrite of the answer already committed to.
///
/// The guard is never held across an await point; every method locks, reads
/// or writes, and drops the guard before returning. A poisoned lock means a
/// tool body already panicked and the run is over either way, so the inner
/// value is recovered rather than turned into a second failure.
pub struct TerminalSlot<T>(Arc<Mutex<Option<T>>>);

impl<T> TerminalSlot<T> {
    /// Create an empty slot.
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }

    /// Record the run's answer.
    ///
    /// # Errors
    ///
    /// Returns [`AlreadyRecorded`] when the slot is filled; the recorded
    /// value stands.
    pub fn record(&self, value: T) -> Result<(), AlreadyRecorded> {
        let mut slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.is_some() {
            return Err(AlreadyRecorded);
        }
        *slot = Some(value);
        Ok(())
    }

    /// Whether an answer has been recorded.
    pub fn is_recorded(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }
}

impl<T: Clone> TerminalSlot<T> {
    /// The recorded answer, if the run committed to one.
    pub fn recorded(&self) -> Option<T> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl<T> Default for TerminalSlot<T> {
    fn default() -> Self {
        Self::new()
    }
}

// Derived `Clone` would demand `T: Clone`, but cloning a slot handle only
// clones the `Arc` - the payload is never copied.
impl<T> Clone for TerminalSlot<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for TerminalSlot<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TerminalSlot")
            .field(&self.0.lock().unwrap_or_else(PoisonError::into_inner))
            .finish()
    }
}
