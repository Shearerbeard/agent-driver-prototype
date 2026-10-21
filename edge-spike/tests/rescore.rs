//! Offline rescore: recompute flags + verdicts from saved answers.
//!
//! Builds a fixture run directory (corpus.json + results.jsonl), rescored
//! without touching the scorer, and asserts the policy change flips an
//! excerpt-only truncation flag while keeping a worker_output flag.

use std::io::Write;
use std::path::Path;

use edge_spike::arms::SelectionArm;
use edge_spike::claim::GroundingStats;
use edge_spike::corpus::{CapturedEdge, Corpus, FailureClass, LabelledEdge, ToolCall};
use edge_spike::runner::{RESULTS_JSONL, ScoredEdge, rescore_run_dir};
use edge_spike::state::{PlanContext, PlanStep, WorkerOutput};
use edge_spike::verdict::{
    AnswerSummary, EvidenceSummary, StageLatencies, TokenUsage, TruncationFlags, Verdict,
    VerdictArtifact, Weights,
};

/// Capture long enough that the head excerpt (a quarter of the budget) is
/// bounded, with the worker claims placed at the very start of the capture.
fn edge_with_claims_at_start(worker_truncated: bool) -> CapturedEdge {
    CapturedEdge {
        scenario: "rescore".to_owned(),
        iteration: 0,
        task_id: 1,
        worker_id: "w1".to_owned(),
        goal: "goal".to_owned(),
        plan_step: PlanStep {
            task: "task".to_owned(),
            worker_role: "worker".to_owned(),
            depends_on: Vec::new(),
        },
        plan_context: PlanContext {
            consumers: Vec::new(),
            siblings: Vec::new(),
        },
        context_provided: String::new(),
        worker_output: WorkerOutput {
            summary: "summary".to_owned(),
            result: "task_HEAD00001 150ms".to_owned(),
            truncated: worker_truncated,
        },
        tool_calls: vec![ToolCall {
            tool_call_id: "tc1".to_owned(),
            tool_name: "query".to_owned(),
            arguments: serde_json::json!({}),
            result: format!("task_HEAD00001 150ms\n{}", "z".repeat(8_000)),
            success: true,
        }],
        confidence: None,
    }
}

fn labelled(variant: &str, edge: CapturedEdge) -> LabelledEdge {
    LabelledEdge {
        edge,
        class: FailureClass::Clean,
        variant_id: variant.to_owned(),
    }
}

fn best_answers() -> AnswerSummary {
    AnswerSummary {
        evidence_presence: 0.95,
        task_fidelity: 0.0,
        goal_alignment: 0.0,
        evidence_grounding: 0.0,
        handoff_integrity: 0.9,
        scope_discipline: 0.9,
    }
}

fn old_artifact(flags: TruncationFlags) -> VerdictArtifact {
    VerdictArtifact {
        rubric_version: "v2".to_owned(),
        weights_version: "v2-polarity-fixed".to_owned(),
        weights: Weights::default(),
        resolved_model_id: "jev-1.13.0".to_owned(),
        state_hash: "hash".to_owned(),
        state_chars: 1,
        answers: best_answers(),
        grounding_stats: GroundingStats::default(),
        composite: 0.9,
        verdict: Verdict::Flag {
            dims: vec!["truncation".to_owned()],
        },
        truncation_flags: flags,
        evidence_summary: EvidenceSummary::default(),
        verifier_error: None,
        latency_ms: StageLatencies::default(),
        tokens: TokenUsage::default(),
        timestamp: "0".to_owned(),
    }
}

fn write_fixture(dir: &Path) {
    let corpus = Corpus {
        calibration: vec![
            labelled("resc-cover", edge_with_claims_at_start(false)),
            labelled("resc-worker", edge_with_claims_at_start(true)),
        ],
        held_out: Vec::new(),
    };
    std::fs::write(
        dir.join("corpus.json"),
        serde_json::to_string_pretty(&corpus).expect("serialize corpus"),
    )
    .expect("write corpus.json");

    let rows = [
        ScoredEdge {
            variant_id: "resc-cover".to_owned(),
            class: FailureClass::Clean,
            arm: SelectionArm::HeadTruncation,
            // Old semantics: a bounded excerpt set `evidence`.
            artifact: old_artifact(TruncationFlags {
                evidence: true,
                source: true,
                ..TruncationFlags::default()
            }),
        },
        ScoredEdge {
            variant_id: "resc-worker".to_owned(),
            class: FailureClass::Clean,
            arm: SelectionArm::HeadTruncation,
            artifact: old_artifact(TruncationFlags {
                evidence: true,
                worker_output: true,
                source: true,
                ..TruncationFlags::default()
            }),
        },
    ];

    let mut file = std::fs::File::create(dir.join(RESULTS_JSONL)).expect("create results.jsonl");
    for row in &rows {
        writeln!(
            file,
            "{}",
            serde_json::to_string(row).expect("serialize row")
        )
        .expect("write row");
    }
}

#[tokio::test]
async fn rescore_flips_excerpt_only_flag_and_keeps_worker_output_flag() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_fixture(dir.path());

    let outcome = rescore_run_dir(dir.path())
        .await
        .unwrap_or_else(|e| panic!("rescore: {e}"));

    assert_eq!(outcome.deltas.len(), 2, "both rows are rescored");

    let cover = outcome
        .deltas
        .iter()
        .find(|d| d.variant_id == "resc-cover")
        .expect("cover delta");
    assert_eq!(
        cover.old_verdict,
        Verdict::Flag {
            dims: vec!["truncation".to_owned()]
        }
    );
    assert_eq!(
        cover.new_verdict,
        Verdict::Pass,
        "an excerpt-only truncation must flip to the dimension-clean verdict"
    );
    assert!(!cover.new_flags.evidence);
    assert!(cover.new_flags.evidence_excerpted);

    let worker = outcome
        .deltas
        .iter()
        .find(|d| d.variant_id == "resc-worker")
        .expect("worker delta");
    assert_eq!(
        worker.new_verdict,
        Verdict::Flag {
            dims: vec!["truncation".to_owned()]
        },
        "worker_output truncation must stay verdict-driving"
    );
    assert!(worker.new_flags.worker_output);

    // The written artifact carries the new flags, verdict, and a populated
    // compact evidence summary.
    let written = std::fs::read_to_string(&outcome.written).expect("read rescored");
    let rows: Vec<ScoredEdge> = written
        .lines()
        .map(|line| serde_json::from_str(line).expect("parse rescored row"))
        .collect();
    assert_eq!(rows.len(), 2);
    let cover_row = rows
        .iter()
        .find(|r| r.variant_id == "resc-cover")
        .expect("cover row");
    assert_eq!(cover_row.artifact.verdict, Verdict::Pass);
    assert!(!cover_row.artifact.truncation_flags.evidence);
    assert!(cover_row.artifact.truncation_flags.evidence_excerpted);
    assert!(
        !cover_row.artifact.evidence_summary.regions.is_empty(),
        "the artifact must record per-region coverage rows"
    );
}
