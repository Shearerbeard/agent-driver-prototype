//! Versioned verdict artifact types and composite scoring.

use serde::{Deserialize, Serialize};

use crate::claim::GroundingStats;
use crate::rubric::EdgeRubricAnswers;
use crate::state::{ClaimEvidenceV2, EdgeState, FoundIn};

/// Final verdict enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Pass,
    Flag { dims: Vec<String> },
    InsufficientEvidence,
    VerifierError,
}

/// Typed reason for a verifier-level error verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifierErrorReason {
    /// The scorer API rejected the request (non-retryable 4xx).
    RequestRejected,
}

/// Typed detail carried by a verifier-error artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifierErrorDetail {
    pub reason: VerifierErrorReason,
    pub detail: String,
}

/// Serializable answer summary extracted from typed answers.
///
/// Ordinal scores (`task_fidelity`, `goal_alignment`, `evidence_grounding`)
/// are probability-weighted level INDICES. Because `rubric.rs` declares the
/// best variant first (e.g. `TaskFidelity::Excellent`), 0 = best and higher
/// = worse. The composite scoring function inverts these dimensions so that
/// a higher composite means better quality.
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
    /// Serialized length of the assembled state actually sent.
    pub state_chars: usize,
    pub answers: AnswerSummary,
    pub grounding_stats: GroundingStats,
    pub composite: f64,
    pub verdict: Verdict,
    pub truncation_flags: TruncationFlags,
    /// Compact region/claim coverage summary. Informational only; serde
    /// defaulted so artifacts written before the field existed still parse.
    #[serde(default)]
    pub evidence_summary: EvidenceSummary,
    /// Set only when the verdict is [`Verdict::VerifierError`].
    #[serde(default)]
    pub verifier_error: Option<VerifierErrorDetail>,
    pub latency_ms: StageLatencies,
    pub tokens: TokenUsage,
    pub timestamp: String,
}

/// Compact per-region evidence row recorded on the artifact.
///
/// Deliberately excludes the excerpt text: the report needs the size and
/// coverage of each region, not its bytes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EvidenceRegionSummary {
    pub region_id: usize,
    pub tool_name: String,
    pub full_region_chars: usize,
    pub excerpt_chars: usize,
    pub source_truncated: bool,
    pub truncated: bool,
    /// Supported claims referencing this region that the shown excerpt does
    /// not cover.
    pub uncovered_hits: usize,
}

/// Compact evidence summary for one scored edge.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EvidenceSummary {
    pub regions: Vec<EvidenceRegionSummary>,
    pub claims_total: usize,
    pub claims_supported: usize,
}

impl From<&ClaimEvidenceV2> for EvidenceSummary {
    fn from(evidence: &ClaimEvidenceV2) -> Self {
        let regions = evidence
            .regions
            .iter()
            .map(|r| EvidenceRegionSummary {
                region_id: r.region_id,
                tool_name: r.tool_name.clone(),
                full_region_chars: r.full_region_chars,
                excerpt_chars: r.excerpt.as_ref().map_or(0, |e| e.chars().count()),
                source_truncated: r.source_truncated,
                truncated: r.truncated,
                uncovered_hits: r.uncovered_hits,
            })
            .collect();
        let claims_supported = evidence
            .claims
            .iter()
            .filter(|c| c.found_in != FoundIn::NotFound)
            .count();
        Self {
            regions,
            claims_total: evidence.claims.len(),
            claims_supported,
        }
    }
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
    // Ordinal dimension max indices (levels - 1). These are frozen into the
    // artifact for transparency, but the scoring math uses the rubric-derived
    // constants so old artifacts with the buggy level-count denominators are
    // corrected at read time.
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
            fidelity_levels: 4.0,
            alignment_levels: 2.0,
            grounding_levels: 3.0,
        }
    }
}

// Max indices for the ordinal rubric dimensions. `rubric.rs` declares the
// best variant first, so a `JevScore::score()` value of 0 means best quality
// and the max index means worst. The composite inverts each dimension via
// `(max_index - score) / max_index` so that higher composite = better quality.
const FIDELITY_MAX_INDEX: f64 = 4.0;
const ALIGNMENT_MAX_INDEX: f64 = 2.0;
const GROUNDING_MAX_INDEX: f64 = 3.0;

/// Per-field truncation flags.
///
/// The six named verdict-driving fields record **assembly**-time information
/// loss: a bounded field the arm's state assembly actually capped. `source` is
/// a separate provenance flag for truncation that pre-existed in the capture
/// (the C0 ` [truncated]` marker on a worker output or tool result); it is
/// **upstream-capture provenance, never verdict-driving**. `evidence` is true
/// only when a supported claim's hit positions fall outside the shown region
/// excerpt (real information loss), not merely when an excerpt is bounded.
/// `evidence_excerpted` records that latter case (a bounded excerpt that still
/// covers every claim hit) and is informational only.
///
/// [`TruncationFlags::any`] ignores both `source` and `evidence_excerpted`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TruncationFlags {
    pub goal: bool,
    pub plan_step: bool,
    pub plan_context: bool,
    pub worker_output: bool,
    pub context_provided: bool,
    /// Real evidence loss: a supported claim hit outside its region excerpt.
    pub evidence: bool,
    /// Informational: some region excerpt is shorter than the full region.
    #[serde(default)]
    pub evidence_excerpted: bool,
    /// Capture-level (pre-existing) truncation marker, not an assembly cap.
    pub source: bool,
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

/// Composite scoring from a saved [`AnswerSummary`].
///
/// Ordinal dimensions are inverted because `JevScore::score()` returns a
/// probability-weighted level index where 0 is the best variant and the max
/// index is the worst (see `rubric.rs` declaration order). Noul dimensions are
/// already "higher is better" and are left unchanged.
pub fn score_composite_from_summary(
    answers: &AnswerSummary,
    weights: &Weights,
    truncation: &TruncationFlags,
) -> (f64, Vec<String>) {
    let fidelity = normalize_ordinal(answers.task_fidelity, FIDELITY_MAX_INDEX);
    let alignment = normalize_ordinal(answers.goal_alignment, ALIGNMENT_MAX_INDEX);
    let grounding = normalize_ordinal(answers.evidence_grounding, GROUNDING_MAX_INDEX);
    let handoff = answers.handoff_integrity;
    let scope = answers.scope_discipline;

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

/// Composite scoring with code-side weights and floor rules.
pub fn score_composite(
    answers: &EdgeRubricAnswers,
    weights: &Weights,
    truncation: &TruncationFlags,
) -> (f64, Vec<String>) {
    let summary = AnswerSummary::from(answers);
    score_composite_from_summary(&summary, weights, truncation)
}

impl TruncationFlags {
    /// True if any verdict-driving field is truncated.
    ///
    /// `source` (upstream-capture provenance) and `evidence_excerpted`
    /// (bounded-but-covered budgeting) are deliberately excluded.
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

fn normalize_ordinal(score: f64, max_index: f64) -> f64 {
    ((max_index - score) / max_index).clamp(0.0, 1.0)
}

/// Map a saved answer summary + composite to a verdict enum.
pub fn resolve_verdict_from_summary(
    summary: &AnswerSummary,
    composite: f64,
    flags: &[String],
    weights: &Weights,
) -> Verdict {
    if summary.evidence_presence < weights.evidence_presence_threshold {
        return Verdict::InsufficientEvidence;
    }
    if composite < weights.pass_threshold || !flags.is_empty() {
        return Verdict::Flag {
            dims: flags.to_vec(),
        };
    }
    Verdict::Pass
}

/// Map answers + composite to a verdict enum using thresholds from `weights`.
pub fn resolve_verdict(
    answers: &EdgeRubricAnswers,
    composite: f64,
    flags: &[String],
    weights: &Weights,
) -> Verdict {
    let summary = AnswerSummary::from(answers);
    resolve_verdict_from_summary(&summary, composite, flags, weights)
}

/// Recompute the composite and verdict for a saved artifact from its raw
/// per-question answers. This corrects the polarity bug in older artifacts
/// without re-running the scorer.
pub fn recompute_artifact(artifact: &mut VerdictArtifact) {
    // Normalize stored weights to the corrected max indices in case this is an
    // older artifact that still carries the buggy level-count denominators.
    artifact.weights.fidelity_levels = FIDELITY_MAX_INDEX;
    artifact.weights.alignment_levels = ALIGNMENT_MAX_INDEX;
    artifact.weights.grounding_levels = GROUNDING_MAX_INDEX;

    let (composite, flags) = score_composite_from_summary(
        &artifact.answers,
        &artifact.weights,
        &artifact.truncation_flags,
    );
    artifact.composite = composite;
    artifact.verdict =
        resolve_verdict_from_summary(&artifact.answers, composite, &flags, &artifact.weights);
    artifact.weights_version = "v2-polarity-fixed".to_owned();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_ordinal_answers_score_higher_than_worst() {
        let weights = Weights::default();
        let truncation = TruncationFlags::default();

        let best = AnswerSummary {
            evidence_presence: 0.9,
            task_fidelity: 0.0,      // Excellent
            goal_alignment: 0.0,     // Aligned
            evidence_grounding: 0.0, // Supported
            handoff_integrity: 0.9,
            scope_discipline: 0.9,
        };
        let worst = AnswerSummary {
            evidence_presence: 0.9,
            task_fidelity: 4.0,      // None
            goal_alignment: 2.0,     // Misaligned
            evidence_grounding: 3.0, // UnsupportedOrFabricated
            handoff_integrity: 0.1,
            scope_discipline: 0.1,
        };

        let (best_score, best_flags) = score_composite_from_summary(&best, &weights, &truncation);
        let (worst_score, worst_flags) =
            score_composite_from_summary(&worst, &weights, &truncation);

        assert!(
            best_score > worst_score,
            "best ordinal answers should composite higher than worst: {best_score} vs {worst_score}"
        );
        assert!(
            best_flags.is_empty(),
            "best answers should not trigger floor rules: {best_flags:?}"
        );
        assert!(
            worst_flags.contains(&"handoff_integrity".to_owned()),
            "worst handoff should be flagged: {worst_flags:?}"
        );
        assert!(
            worst_flags.contains(&"scope_discipline".to_owned()),
            "worst scope should be flagged: {worst_flags:?}"
        );
    }

    #[test]
    fn fixture_vector_composite_matches_expected_value() {
        let summary = AnswerSummary {
            evidence_presence: 0.9,
            task_fidelity: 3.27,
            goal_alignment: 1.50,
            evidence_grounding: 1.89,
            handoff_integrity: 0.09,
            scope_discipline: 0.07,
        };
        let weights = Weights::default();
        let (composite, flags) =
            score_composite_from_summary(&summary, &weights, &TruncationFlags::default());

        assert!(
            (composite - 0.218_125).abs() < 1e-6,
            "expected corrected composite ~0.218, got {composite}"
        );
        assert!(
            flags.contains(&"handoff_integrity".to_owned()),
            "handoff below threshold should be flagged: {flags:?}"
        );
        assert!(
            flags.contains(&"scope_discipline".to_owned()),
            "scope below threshold should be flagged: {flags:?}"
        );
    }

    #[test]
    fn saved_artifact_recompute_ignores_buggy_denominators_and_composite() {
        use crate::claim::GroundingStats;

        let mut artifact = VerdictArtifact {
            rubric_version: "v2".to_owned(),
            weights_version: "v2-provisional".to_owned(),
            weights: Weights {
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
                // Buggy level-count denominators as stored in old artifacts.
                fidelity_levels: 5.0,
                alignment_levels: 3.0,
                grounding_levels: 4.0,
            },
            resolved_model_id: "jev-1.13.0-replay".to_owned(),
            state_hash: "abc".to_owned(),
            state_chars: 0,
            answers: AnswerSummary {
                evidence_presence: 0.9,
                task_fidelity: 3.27,
                goal_alignment: 1.50,
                evidence_grounding: 1.89,
                handoff_integrity: 0.09,
                scope_discipline: 0.07,
            },
            grounding_stats: GroundingStats::default(),
            // Buggy stored composite; should be overwritten from answers.
            composite: 0.404_25,
            verdict: Verdict::Pass,
            truncation_flags: TruncationFlags::default(),
            evidence_summary: EvidenceSummary::default(),
            verifier_error: None,
            latency_ms: StageLatencies::default(),
            tokens: TokenUsage::default(),
            timestamp: "0".to_owned(),
        };

        recompute_artifact(&mut artifact);

        assert!(
            (artifact.composite - 0.218_125).abs() < 1e-6,
            "recomputed composite should ignore stored buggy composite/denominators, got {}",
            artifact.composite
        );
        match &artifact.verdict {
            Verdict::Flag { dims } => {
                assert!(dims.contains(&"handoff_integrity".to_owned()));
                assert!(dims.contains(&"scope_discipline".to_owned()));
            }
            other => panic!("expected Flag verdict after recompute, got {other:?}"),
        }
        assert_eq!(artifact.weights_version, "v2-polarity-fixed");
        assert_eq!(artifact.weights.fidelity_levels, 4.0);
        assert_eq!(artifact.weights.alignment_levels, 2.0);
        assert_eq!(artifact.weights.grounding_levels, 3.0);
    }

    #[test]
    fn evidence_excerpted_is_informational_only() {
        let flags = TruncationFlags {
            evidence_excerpted: true,
            source: true,
            ..TruncationFlags::default()
        };
        assert!(
            !flags.any(),
            "excerpted/source must not drive the truncation flag"
        );

        let summary = AnswerSummary {
            evidence_presence: 0.9,
            task_fidelity: 0.0,
            goal_alignment: 0.0,
            evidence_grounding: 0.0,
            handoff_integrity: 0.9,
            scope_discipline: 0.9,
        };
        let weights = Weights::default();
        let (composite, dims) = score_composite_from_summary(&summary, &weights, &flags);
        assert!(
            !dims.contains(&"truncation".to_owned()),
            "informational flags must not add a truncation dim: {dims:?}"
        );
        assert_eq!(
            resolve_verdict_from_summary(&summary, composite, &dims, &weights),
            Verdict::Pass
        );
    }

    #[test]
    fn old_artifact_without_new_fields_still_deserializes() {
        let artifact = VerdictArtifact {
            rubric_version: "v2".to_owned(),
            weights_version: "v2-polarity-fixed".to_owned(),
            weights: Weights::default(),
            resolved_model_id: "jev-1.13.0".to_owned(),
            state_hash: "hash".to_owned(),
            state_chars: 1,
            answers: AnswerSummary {
                evidence_presence: 0.9,
                task_fidelity: 0.0,
                goal_alignment: 0.0,
                evidence_grounding: 0.0,
                handoff_integrity: 0.9,
                scope_discipline: 0.9,
            },
            grounding_stats: GroundingStats::default(),
            composite: 0.9,
            verdict: Verdict::Pass,
            truncation_flags: TruncationFlags {
                evidence: true,
                source: true,
                ..TruncationFlags::default()
            },
            evidence_summary: EvidenceSummary::default(),
            verifier_error: None,
            latency_ms: StageLatencies::default(),
            tokens: TokenUsage::default(),
            timestamp: "0".to_owned(),
        };

        // Strip the two fields this change added to mimic a pre-change line.
        let mut value = serde_json::to_value(&artifact).expect("serialize");
        value
            .as_object_mut()
            .expect("artifact object")
            .remove("evidence_summary");
        value["truncation_flags"]
            .as_object_mut()
            .expect("flags object")
            .remove("evidence_excerpted");

        let parsed: VerdictArtifact = serde_json::from_value(value).expect("old artifact parses");
        assert_eq!(parsed.verdict, Verdict::Pass);
        assert!(parsed.truncation_flags.evidence);
        assert!(!parsed.truncation_flags.evidence_excerpted);
        assert!(parsed.evidence_summary.regions.is_empty());
    }
}
