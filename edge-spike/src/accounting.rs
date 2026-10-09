//! Latency and token accounting scaffolding.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::arms::SelectionArm;
use crate::runner::ScoredEdge;
use crate::verdict::{StageLatencies, Verdict};

/// Aggregate accounting for one arm.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArmAccounting {
    pub arm: String,
    pub edges: usize,
    pub p50_total_ms: u64,
    pub p95_total_ms: u64,
    pub p50_jev_ms: u64,
    pub p95_jev_ms: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub truncation_count: usize,
    pub insufficient_evidence_count: usize,
    pub verifier_error_count: usize,
}

/// Aggregate accounting table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Accounting {
    pub by_arm: BTreeMap<String, ArmAccounting>,
}

impl Accounting {
    /// Build accounting from scored edges.
    #[must_use]
    pub fn from_scored(scored: &[ScoredEdge]) -> Self {
        let mut by_arm: BTreeMap<String, Vec<&ScoredEdge>> = BTreeMap::new();
        for edge in scored {
            by_arm
                .entry(edge.arm.name().to_owned())
                .or_default()
                .push(edge);
        }

        let mut out = BTreeMap::new();
        for (name, edges) in by_arm {
            let mut totals: Vec<&StageLatencies> =
                edges.iter().map(|e| &e.artifact.latency_ms).collect();
            totals.sort_by_key(|l| l.total_ms);
            let p50_total = percentile(&totals, 0.5, |l| l.total_ms);
            let p95_total = percentile(&totals, 0.95, |l| l.total_ms);
            let mut jevs: Vec<&StageLatencies> =
                edges.iter().map(|e| &e.artifact.latency_ms).collect();
            jevs.sort_by_key(|l| l.jev_ms);
            let p50_jev = percentile(&jevs, 0.5, |l| l.jev_ms);
            let p95_jev = percentile(&jevs, 0.95, |l| l.jev_ms);

            let truncation_count = edges
                .iter()
                .filter(|e| {
                    let t = &e.artifact.truncation_flags;
                    t.goal
                        || t.plan_step
                        || t.plan_context
                        || t.worker_output
                        || t.context_provided
                        || t.evidence
                })
                .count();
            let insufficient = edges
                .iter()
                .filter(|e| {
                    matches!(
                        e.artifact.verdict,
                        crate::verdict::Verdict::InsufficientEvidence
                    )
                })
                .count();
            let verifier_errors = edges
                .iter()
                .filter(|e| e.artifact.verifier_error.is_some())
                .count();

            out.insert(
                name.clone(),
                ArmAccounting {
                    arm: name,
                    edges: edges.len(),
                    p50_total_ms: p50_total,
                    p95_total_ms: p95_total,
                    p50_jev_ms: p50_jev,
                    p95_jev_ms: p95_jev,
                    total_input_tokens: edges.iter().map(|e| e.artifact.tokens.input_tokens).sum(),
                    total_output_tokens: edges
                        .iter()
                        .map(|e| e.artifact.tokens.output_tokens)
                        .sum(),
                    truncation_count,
                    insufficient_evidence_count: insufficient,
                    verifier_error_count: verifier_errors,
                },
            );
        }

        Self { by_arm: out }
    }
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "percentile index computation on small arrays"
)]
fn percentile<T>(sorted: &[T], p: f64, f: impl Fn(&T) -> u64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    f(&sorted[idx.clamp(0, sorted.len() - 1)])
}

/// Accuracy summary per arm and class.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccuracySummary {
    pub by_arm_class: BTreeMap<String, BTreeMap<String, Vec<f64>>>,
}

/// Result of the pairwise clean-vs-failure comparison for one arm.
#[derive(Debug, Clone, Copy, Default)]
pub struct PairwiseResult {
    pub rate: f64,
    pub total_pairs: usize,
    pub clean_wins: usize,
    pub truncation_ties: usize,
}

impl AccuracySummary {
    /// Build from scored edges.
    #[must_use]
    pub fn from_scored(scored: &[ScoredEdge]) -> Self {
        let mut map: BTreeMap<String, BTreeMap<String, Vec<f64>>> = BTreeMap::new();
        for edge in scored {
            // A verifier error carries a placeholder composite of 0.0 because no
            // scorer response was received. It is counted separately by
            // `ArmAccounting::verifier_error_count`; folding it into a class
            // bucket would drag clean below every failure in the pairwise metric.
            if matches!(edge.artifact.verdict, Verdict::VerifierError) {
                continue;
            }
            map.entry(edge.arm.name().to_owned())
                .or_default()
                .entry(edge.class.as_str().to_owned())
                .or_default()
                .push(edge.artifact.composite);
        }
        Self { by_arm_class: map }
    }

    /// Pairwise ordering: fraction of (clean, failure) pairs where clean composite > failure composite.
    ///
    /// Board ruling (2026-10-08): keep `truncation` in this metric. A truncated
    /// edge can still carry high-fidelity content; the truncation floor rule is
    /// the designated catcher, not the composite score. We therefore report the
    /// truncation-tie count separately so readers can see how much of the rate
    /// is driven by ties rather than inversions.
    #[must_use]
    pub fn pairwise_clean_vs_failure(&self, arm: SelectionArm) -> PairwiseResult {
        let key = arm.name();
        let Some(classes) = self.by_arm_class.get(key) else {
            return PairwiseResult::default();
        };
        let clean = classes.get("clean").cloned().unwrap_or_default();
        let mut total = 0usize;
        let mut correct = 0usize;
        let mut truncation_ties = 0usize;
        // BaselineBlocked is neither a clean positive nor a failure class for the
        // pairwise metric; it is reported in its own row in the corpus table.
        for class_name in classes.keys() {
            if class_name == "clean"
                || class_name == "legitimate_scope_overlap"
                || class_name == "baseline-blocked"
            {
                continue;
            }
            if let Some(failure) = classes.get(class_name) {
                for c in &clean {
                    for f in failure {
                        total += 1;
                        if c > f {
                            correct += 1;
                        } else if class_name == "truncation" && (c - f).abs() < f64::EPSILON {
                            truncation_ties += 1;
                        }
                    }
                }
            }
        }
        if total == 0 {
            return PairwiseResult {
                rate: 1.0,
                total_pairs: 0,
                clean_wins: 0,
                truncation_ties: 0,
            };
        }
        PairwiseResult {
            rate: f64::from(correct as u32) / f64::from(total as u32),
            total_pairs: total,
            clean_wins: correct,
            truncation_ties,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claim::GroundingStats;
    use crate::corpus::FailureClass;
    use crate::verdict::{
        AnswerSummary, TokenUsage, TruncationFlags, VerdictArtifact, VerifierErrorDetail,
        VerifierErrorReason, Weights,
    };

    fn artifact(composite: f64, verdict: Verdict) -> VerdictArtifact {
        let verifier_error =
            matches!(verdict, Verdict::VerifierError).then(|| VerifierErrorDetail {
                reason: VerifierErrorReason::RequestRejected,
                detail: "400".to_owned(),
            });
        VerdictArtifact {
            rubric_version: "v2".to_owned(),
            weights_version: "v2-polarity-fixed".to_owned(),
            weights: Weights::default(),
            resolved_model_id: "test".to_owned(),
            state_hash: "hash".to_owned(),
            state_chars: 0,
            answers: AnswerSummary {
                evidence_presence: 0.0,
                task_fidelity: 0.0,
                goal_alignment: 0.0,
                evidence_grounding: 0.0,
                handoff_integrity: 0.0,
                scope_discipline: 0.0,
            },
            grounding_stats: GroundingStats::default(),
            composite,
            verdict,
            truncation_flags: TruncationFlags::default(),
            verifier_error,
            latency_ms: StageLatencies::default(),
            tokens: TokenUsage::default(),
            timestamp: "0".to_owned(),
        }
    }

    fn scored(class: FailureClass, composite: f64, verdict: Verdict) -> ScoredEdge {
        ScoredEdge {
            variant_id: "v".to_owned(),
            class,
            arm: SelectionArm::ClaimIndexed,
            artifact: artifact(composite, verdict),
        }
    }

    #[test]
    fn verifier_errors_are_excluded_from_class_buckets_but_still_counted() {
        let scored = vec![
            scored(FailureClass::Clean, 0.9, Verdict::Pass),
            scored(FailureClass::Clean, 0.8, Verdict::Pass),
            scored(FailureClass::Clean, 0.0, Verdict::VerifierError),
            scored(FailureClass::Drifted, 0.4, Verdict::Pass),
        ];

        let accuracy = AccuracySummary::from_scored(&scored);
        let classes = accuracy
            .by_arm_class
            .get(SelectionArm::ClaimIndexed.name())
            .expect("claim_indexed accuracy");
        let clean = classes.get("clean").expect("clean bucket");
        assert_eq!(
            clean.as_slice(),
            [0.9, 0.8],
            "verifier-error composite must not enter the clean bucket"
        );

        let accounting = Accounting::from_scored(&scored);
        let arm_acc = accounting
            .by_arm
            .get(SelectionArm::ClaimIndexed.name())
            .expect("claim_indexed accounting");
        assert_eq!(arm_acc.verifier_error_count, 1);

        let pairwise = accuracy.pairwise_clean_vs_failure(SelectionArm::ClaimIndexed);
        assert_eq!(pairwise.total_pairs, 2, "one pair per clean composite");
        assert_eq!(
            pairwise.clean_wins, 2,
            "both clean composites beat the failure"
        );
        assert!(
            (pairwise.rate - 1.0).abs() < f64::EPSILON,
            "the 0.0 placeholder must not appear in any pair"
        );
    }
}
