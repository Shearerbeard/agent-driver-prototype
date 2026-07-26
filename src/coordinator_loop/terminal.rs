//! The answer a run commits to, and the first-write-wins slot that holds it.

use std::sync::{Arc, Mutex};

use crate::context::{EvidenceText, WorkerClaim};

use super::error::{AlreadyRecorded, CoordinatorLoopError};

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
        _response: &str,
        _response_summary: Option<&str>,
    ) -> Result<Self, CoordinatorLoopError> {
        todo!("S71 Phase 2")
    }

    /// The answer text delivered to the user.
    pub fn response(&self) -> &str {
        todo!("S71 Phase 2")
    }

    /// The model's own short-form gloss of the answer, when it wrote one.
    pub fn response_summary(&self) -> Option<&str> {
        todo!("S71 Phase 2")
    }
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
    pub fn new(_claim: WorkerClaim, _result: EvidenceText) -> Self {
        todo!("S71 Phase 2")
    }

    /// The worker's own summary and confidence.
    pub fn claim(&self) -> &WorkerClaim {
        todo!("S71 Phase 2")
    }

    /// The result body the claim is made about.
    pub fn result(&self) -> &EvidenceText {
        todo!("S71 Phase 2")
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
/// The guard is never held across an await point; every method locks,
/// reads or writes, and drops the guard before returning.
pub struct TerminalSlot<T>(Arc<Mutex<Option<T>>>);

impl<T> TerminalSlot<T> {
    /// Create an empty slot.
    pub fn new() -> Self {
        todo!("S71 Phase 2")
    }

    /// Record the run's answer.
    ///
    /// # Errors
    ///
    /// Returns [`AlreadyRecorded`] when the slot is filled; the recorded
    /// value stands.
    pub fn record(&self, _value: T) -> Result<(), AlreadyRecorded> {
        todo!("S71 Phase 2")
    }

    /// Whether an answer has been recorded.
    pub fn is_recorded(&self) -> bool {
        todo!("S71 Phase 2")
    }
}

impl<T: Clone> TerminalSlot<T> {
    /// The recorded answer, if the run committed to one.
    pub fn recorded(&self) -> Option<T> {
        todo!("S71 Phase 2")
    }
}

impl<T> Default for TerminalSlot<T> {
    fn default() -> Self {
        todo!("S71 Phase 2")
    }
}

// Derived `Clone` would demand `T: Clone`, but cloning a slot handle only
// clones the `Arc` — the payload is never copied.
impl<T> Clone for TerminalSlot<T> {
    fn clone(&self) -> Self {
        todo!("S71 Phase 2")
    }
}
