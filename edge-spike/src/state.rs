//! Edge verifier state types.

use serde::{Deserialize, Serialize};

/// One plan step as it appears in the scorer state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub task: String,
    pub worker_role: String,
    pub depends_on: Vec<usize>,
}

/// Consumer and sibling task descriptions from the plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanContext {
    pub consumers: Vec<TaskRef>,
    pub siblings: Vec<TaskRef>,
}

/// A task reference inside plan context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRef {
    pub task: String,
}

/// Worker output WITHOUT self-confidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerOutput {
    pub summary: String,
    pub result: String,
    pub truncated: bool,
}

/// Provenance of a claim's support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FoundIn {
    ToolCapture,
    ContextProvided,
    NotFound,
}

/// One extracted claim plus its bounded evidence window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimEvidence {
    pub claim: String,
    pub found_in: FoundIn,
    pub window: Option<String>,
    pub truncated: bool,
}

/// The full state payload sent to Jev for one edge.
#[derive(Debug, Clone, Serialize, Deserialize, jev_driver::prelude::JevState)]
#[jev(describe = "Edge verifier state for one submitted worker output")]
pub struct EdgeState {
    pub goal: String,
    pub plan_step: PlanStep,
    pub plan_context: PlanContext,
    pub worker_output: WorkerOutput,
    pub context_provided: String,
    pub claim_evidence: Vec<ClaimEvidence>,
}

impl EdgeState {
    /// Total serialized character length of the state payload.
    #[must_use]
    pub fn char_len(&self) -> usize {
        serde_json::to_string(self)
            .map(|s| s.len())
            .unwrap_or(usize::MAX)
    }
}
