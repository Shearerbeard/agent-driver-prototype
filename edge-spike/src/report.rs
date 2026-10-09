//! Report markdown skeleton generation.

use std::collections::BTreeMap;
use std::path::Path;

use crate::accounting::{Accounting, AccuracySummary};
use crate::arms::SelectionArm;
use crate::corpus::{Corpus, FailureClass};
use crate::runner::ScoredEdge;
use crate::verdict::Verdict;

/// Generate the markdown report skeleton.
pub fn generate_report(
    corpus: &Corpus,
    scored: &[ScoredEdge],
    accounting: &Accounting,
    accuracy: &AccuracySummary,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("# W7 JEV Rubric Research Spike Report".to_owned());
    lines.push(String::new());
    lines.push(
        "> Phase 1 (offline). Thresholds, weights, and replay responses are PROVISIONAL and SYNTHETIC."
            .to_owned(),
    );
    lines.push(String::new());

    lines.push("## Corpus".to_owned());
    lines.push("- Scenarios: captured from aura RCA runs".to_owned());
    lines.push(format!("- Total captured edges: {}", corpus.total() / 11));
    lines.push(format!("- Total labelled variants: {}", corpus.total()));
    lines.push(format!(
        "- Calibration variants: {}",
        corpus.calibration.len()
    ));
    lines.push(format!("- Held-out variants: {}", corpus.held_out.len()));
    lines.push(String::new());

    lines.push("### Variants per class".to_owned());
    lines.push("| class | calibration | held_out |".to_owned());
    lines.push("| --- | --- | --- |".to_owned());
    for (class, (cal, held)) in corpus.class_counts() {
        lines.push(format!("| {class:?} | {cal} | {held} |"));
    }
    lines.push(String::new());

    lines.push("## Latency profile".to_owned());
    lines.push("| arm | edges | p50_total_ms | p95_total_ms | p50_jev_ms | p95_jev_ms | input_tokens | output_tokens | truncations | insufficient_evidence | verifier_errors |".to_owned());
    lines.push("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |".to_owned());
    for acc in accounting.by_arm.values() {
        lines.push(format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            acc.arm,
            acc.edges,
            acc.p50_total_ms,
            acc.p95_total_ms,
            acc.p50_jev_ms,
            acc.p95_jev_ms,
            acc.total_input_tokens,
            acc.total_output_tokens,
            acc.truncation_count,
            acc.insufficient_evidence_count,
            acc.verifier_error_count
        ));
    }
    lines.push(String::new());

    lines.push("## Accuracy summary".to_owned());
    lines.push("Pairwise clean-vs-failure rate and truncation-tie count per arm.".to_owned());
    lines.push(
        "| arm | clean_vs_failure_pairwise | total_pairs | clean_wins | truncation_ties |"
            .to_owned(),
    );
    lines.push("| --- | --- | --- | --- | --- |".to_owned());
    for arm in SelectionArm::all() {
        let result = accuracy.pairwise_clean_vs_failure(*arm);
        lines.push(format!(
            "| {} | {:.2} | {} | {} | {} |",
            arm.name(),
            result.rate,
            result.total_pairs,
            result.clean_wins,
            result.truncation_ties
        ));
    }
    lines.push(String::new());

    lines.push("## Per-class scoring".to_owned());
    lines.push("Average composite score per arm and verdict counts per class.".to_owned());
    lines.push("| class | head_truncation | claim_indexed | flash_summarization | pass | flag | insufficient | verifier_errors | floor_rules |".to_owned());
    lines.push("| --- | --- | --- | --- | --- | --- | --- | --- | --- |".to_owned());
    let by_class = class_summary(scored);
    for class in FailureClass::all() {
        let summary = by_class.get(class).cloned().unwrap_or_default();
        lines.push(format!(
            "| {class:?} | {:.3} | {:.3} | {:.3} | {} | {} | {} | {} | {} |",
            summary
                .composite_by_arm
                .get("head_truncation")
                .copied()
                .unwrap_or(0.0),
            summary
                .composite_by_arm
                .get("claim_indexed")
                .copied()
                .unwrap_or(0.0),
            summary
                .composite_by_arm
                .get("flash_summarization")
                .copied()
                .unwrap_or(0.0),
            summary.pass,
            summary.flag,
            summary.insufficient,
            summary.verifier_error,
            summary.floor_rules.join(", ")
        ));
    }
    lines.push(String::new());

    lines.push("## Notes".to_owned());
    lines.push(
        "- Composite scores are synthetic in this offline phase (class-aware replay responses)."
            .to_owned(),
    );
    lines.push("- Live phase replaces scripted responses with one frozen cloud run.".to_owned());
    lines.push(
        "- All thresholds and normalization denominators are versioned in the frozen `Weights` field of each artifact."
            .to_owned(),
    );
    lines.push(
        "- Arm 1 (head-truncation) is expected to underperform Arm 2 on over-budget edges."
            .to_owned(),
    );
    lines.push(
        "- BaselineBlocked variants are excluded from the pairwise clean-vs-failure metric."
            .to_owned(),
    );
    lines.push(String::new());

    lines.join("\n")
}

#[derive(Debug, Clone, Default)]
struct ClassSummary {
    composite_by_arm: BTreeMap<String, f64>,
    pass: usize,
    flag: usize,
    insufficient: usize,
    verifier_error: usize,
    floor_rules: Vec<String>,
}

fn class_summary(scored: &[ScoredEdge]) -> BTreeMap<FailureClass, ClassSummary> {
    let mut grouped: BTreeMap<FailureClass, Vec<&ScoredEdge>> = BTreeMap::new();
    for edge in scored {
        grouped.entry(edge.class).or_default().push(edge);
    }

    let mut out = BTreeMap::new();
    for (class, edges) in grouped {
        let mut summary = ClassSummary::default();
        let mut by_arm: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        let mut rule_counts: BTreeMap<String, usize> = BTreeMap::new();

        for edge in edges {
            by_arm
                .entry(edge.arm.name().to_owned())
                .or_default()
                .push(edge.artifact.composite);

            match &edge.artifact.verdict {
                Verdict::Pass => summary.pass += 1,
                Verdict::Flag { .. } => summary.flag += 1,
                Verdict::InsufficientEvidence => summary.insufficient += 1,
                Verdict::VerifierError => summary.verifier_error += 1,
            }

            if let Verdict::Flag { dims } = &edge.artifact.verdict {
                for dim in dims {
                    *rule_counts.entry(dim.clone()).or_insert(0) += 1;
                }
            }
        }

        for (arm, values) in by_arm {
            let avg = if values.is_empty() {
                0.0
            } else {
                values.iter().sum::<f64>() / f64::from(values.len() as u32)
            };
            summary.composite_by_arm.insert(arm, avg);
        }

        summary.floor_rules = rule_counts
            .into_iter()
            .map(|(rule, count)| format!("{rule}:{count}"))
            .collect();
        out.insert(class, summary);
    }
    out
}

/// Write the report to disk.
pub fn write_report(report: &str, path: &Path) -> anyhow::Result<()> {
    std::fs::write(path, report)?;
    Ok(())
}
