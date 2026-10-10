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
    /// Claim text was found inside a tool-capture region. The concrete
    /// region is referenced by [`ClaimRef::region_id`].
    ToolCapture,
    ContextProvided,
    NotFound,
}

/// V2 evidence schema: shared regions referenced by claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimEvidenceV2 {
    pub regions: Vec<EvidenceRegion>,
    pub claims: Vec<ClaimRef>,
}

/// One bounded region of the flattened tool capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRegion {
    pub region_id: usize,
    pub tool_call_id: String,
    pub tool_name: String,
    pub args_excerpt: String,
    pub excerpt: Option<String>,
    pub span: (usize, usize),
    pub full_region_chars: usize,
    pub source_truncated: bool,
    pub truncated: bool,
}

/// A claim and its resolved region reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimRef {
    pub claim: String,
    pub found_in: FoundIn,
    pub region_id: Option<usize>,
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
    pub claim_evidence: ClaimEvidenceV2,
}

impl EdgeState {
    /// Total serialized character length of the state payload.
    #[must_use]
    pub fn char_len(&self) -> usize {
        serde_json::to_string(self)
            .map(|s| s.chars().count())
            .unwrap_or(usize::MAX)
    }
}
