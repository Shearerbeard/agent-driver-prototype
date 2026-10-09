//! Versioned verdict artifact types and composite scoring.

use serde::{Deserialize, Serialize};

use crate::claim::GroundingStats;
use crate::rubric::EdgeRubricAnswers;
use crate::state::EdgeState;

/// Final verdict enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Pass,
    Flag { dims: Vec<String> },
    InsufficientEvidence,
    VerifierError,
}

/// Serializable answer summary extracted from typed answers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnswerSummary {
    pub evidence_presence: f64,
    pub task_fidelity: f64,
    pub goal_alignment: f64,
    pub evidence_grounding: f64,
    pub handoff_integrity: f64,
    pub scope_discipline: f64,
}

/// Versioned verdict artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerdictArtifact {
    pub rubric_version: String,
    pub weights_version: String,
    pub weights: Weights,
    pub resolved_model_id: String,
    pub state_hash: String,
    pub answers: AnswerSummary,
    pub grounding_stats: GroundingStats,
    pub composite: f64,
    pub verdict: Verdict,
    pub truncation_flags: TruncationFlags,
    pub latency_ms: StageLatencies,
    pub tokens: TokenUsage,
    pub timestamp: String,
}

/// Composite weights, thresholds, and normalization denominators.
///
/// All values are frozen into the verdict artifact so the live phase can
/// tune scoring as data rather than code.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Weights {
    // Dimension weights.
    pub fidelity: f64,
    pub alignment: f64,
    pub grounding: f64,
    pub handoff: f64,
    pub scope: f64,
    // Decision thresholds.
    pub pass_threshold: f64,
    pub evidence_presence_threshold: f64,
    pub grounding_threshold: f64,
    pub handoff_threshold: f64,
    pub scope_threshold: f64,
    // Score normalization denominators (max_score + 1).
    pub fidelity_levels: f64,
    pub alignment_levels: f64,
    pub grounding_levels: f64,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            fidelity: 0.25,
            alignment: 0.15,
            grounding: 0.30,
            handoff: 0.15,
            scope: 0.15,
            pass_threshold: 0.5,
            evidence_presence_threshold: 0.3,
            grounding_threshold: 0.3,
            handoff_threshold: 0.3,
            scope_threshold: 0.3,
            fidelity_levels: 5.0,
            alignment_levels: 3.0,
            grounding_levels: 4.0,
        }
    }
}

/// Per-field truncation flags.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TruncationFlags {
    pub goal: bool,
    pub plan_step: bool,
    pub plan_context: bool,
    pub worker_output: bool,
    pub context_provided: bool,
    pub evidence: bool,
}

/// Stage-level latency bookkeeping.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StageLatencies {
    pub extraction_ms: u64,
    pub retrieval_ms: u64,
    pub summarization_ms: u64,
    pub jev_ms: u64,
    pub fan_out_ms: u64,
    pub write_ms: u64,
    pub total_ms: u64,
}

/// Token accounting.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Compute a stable hash of the serialized state.
#[must_use]
pub fn hash_state(state: &EdgeState) -> String {
    let json = serde_json::to_string(state).unwrap_or_default();
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest as _;
    hasher.update(json.as_bytes());
    hex::encode(hasher.finalize())
}

impl From<&EdgeRubricAnswers> for AnswerSummary {
    fn from(answers: &EdgeRubricAnswers) -> Self {
        Self {
            evidence_presence: answers.evidence_presence.probability.get(),
            task_fidelity: answers.task_fidelity.score(),
            goal_alignment: answers.goal_alignment.score(),
            evidence_grounding: answers.evidence_grounding.score(),
            handoff_integrity: answers.handoff_integrity.probability.get(),
            scope_discipline: answers.scope_discipline.probability.get(),
        }
    }
}

/// Composite scoring with code-side weights and floor rules.
pub fn score_composite(
    answers: &EdgeRubricAnswers,
    weights: &Weights,
    truncation: &TruncationFlags,
) -> (f64, Vec<String>) {
    let fidelity = normalize_score(answers.task_fidelity.score(), weights.fidelity_levels);
    let alignment = normalize_score(answers.goal_alignment.score(), weights.alignment_levels);
    let grounding = normalize_score(answers.evidence_grounding.score(), weights.grounding_levels);
    let handoff = answers.handoff_integrity.probability.get();
    let scope = answers.scope_discipline.probability.get();

    let composite = fidelity * weights.fidelity
        + alignment * weights.alignment
        + grounding * weights.grounding
        + handoff * weights.handoff
        + scope * weights.scope;

    let mut flagged = Vec::new();
    if grounding < weights.grounding_threshold {
        flagged.push("evidence_grounding".to_owned());
    }
    if handoff < weights.handoff_threshold {
        flagged.push("handoff_integrity".to_owned());
    }
    if scope < weights.scope_threshold {
        flagged.push("scope_discipline".to_owned());
    }
    if truncation.any() {
        flagged.push("truncation".to_owned());
    }

    (composite, flagged)
}

impl TruncationFlags {
    /// True if any field is truncated.
    #[must_use]
    pub fn any(&self) -> bool {
        self.goal
            || self.plan_step
            || self.plan_context
            || self.worker_output
            || self.context_provided
            || self.evidence
    }
}

fn normalize_score(score: f64, levels: f64) -> f64 {
    (score / levels).clamp(0.0, 1.0)
}

/// Map answers + composite to a verdict enum using thresholds from `weights`.
pub fn resolve_verdict(
    answers: &EdgeRubricAnswers,
    composite: f64,
    flags: &[String],
    weights: &Weights,
) -> Verdict {
    if answers.evidence_presence.probability.get() < weights.evidence_presence_threshold {
        return Verdict::InsufficientEvidence;
    }
    if composite < weights.pass_threshold || !flags.is_empty() {
        return Verdict::Flag {
            dims: flags.to_vec(),
        };
    }
    Verdict::Pass
}
