//! Persistence and resume tests for the spike runner.
//!
//! These tests verify that `results.jsonl` is append-only, flushed per line,
//! and used as the primary resume source, and that a torn final line is
//! recovered without panic.

use std::collections::HashSet;
use std::io::Write;

use edge_spike::corpus::{CapturedEdge, Corpus, FailureClass, LabelledEdge};
use edge_spike::runner::{RunnerConfig, RunnerMode, ScoredEdge, run_corpus};
use edge_spike::state::{PlanContext, PlanStep, WorkerOutput};

fn dummy_edge(variant_id: &str, class: FailureClass) -> LabelledEdge {
    LabelledEdge {
        edge: CapturedEdge {
            scenario: "persistence".to_owned(),
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
                result: format!("result for {variant_id}"),
                truncated: false,
            },
            tool_calls: Vec::new(),
            confidence: None,
        },
        class,
        variant_id: variant_id.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn results_jsonl_resume_skips_completed_and_recovers_torn_line() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("temp dir: {e}"));
        let corpus = Corpus {
            calibration: vec![dummy_edge("cal-1", FailureClass::Clean)],
            held_out: vec![dummy_edge("held-1", FailureClass::Drifted)],
        };
        let config = RunnerConfig {
            mode: RunnerMode::Replay,
            ..Default::default()
        };

        // Initial full run writes one line per (variant, arm) pair.
        let result = run_corpus(&corpus, &config, Some(dir.path()))
            .await
            .unwrap_or_else(|e| panic!("initial run failed: {e}"));
        assert_eq!(result.scored.len(), 6, "initial run should score all pairs");

        let results_path = dir.path().join(edge_spike::runner::RESULTS_JSONL);
        let initial_text = std::fs::read_to_string(&results_path)
            .unwrap_or_else(|e| panic!("read results.jsonl: {e}"));
        assert_eq!(
            initial_text.lines().count(),
            6,
            "results.jsonl should contain one line per scored pair"
        );
        for line in initial_text.lines() {
            let _: ScoredEdge =
                serde_json::from_str(line).expect("every initial line should parse");
        }

        // Simulate an interruption: keep the first four complete lines and
        // append a torn fifth line.
        let bytes = std::fs::read(&results_path)
            .unwrap_or_else(|e| panic!("read results.jsonl bytes: {e}"));
        let mut newline_positions = bytes
            .iter()
            .enumerate()
            .filter(|&(_, &b)| b == b'\n')
            .map(|(i, _)| i);
        let after_line_4 = newline_positions.nth(3).expect("at least four lines") + 1;

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&results_path)
            .unwrap_or_else(|e| panic!("open results.jsonl for truncation: {e}"));
        file.write_all(&bytes[..after_line_4])
            .unwrap_or_else(|e| panic!("write truncated results.jsonl: {e}"));
        file.write_all(b"{\"variant_id\":\"held-1\",\"arm\":\"claim_indexed\"")
            .unwrap_or_else(|e| panic!("append torn line: {e}"));
        file.flush()
            .unwrap_or_else(|e| panic!("flush torn line: {e}"));
        drop(file);

        // Re-run should recover without panic and score only the missing pairs.
        let result = run_corpus(&corpus, &config, Some(dir.path()))
            .await
            .unwrap_or_else(|e| panic!("resumed run failed: {e}"));

        let expected_missing: HashSet<_> = [
            ("held-1".to_owned(), "claim_indexed".to_owned()),
            ("held-1".to_owned(), "flash_summarization".to_owned()),
        ]
        .into_iter()
        .collect();
        let actual: HashSet<_> = result
            .scored
            .iter()
            .map(|s| (s.variant_id.clone(), s.arm.name().to_owned()))
            .collect();
        assert_eq!(
            actual, expected_missing,
            "resumed run should score exactly the missing pairs and the torn line"
        );

        let final_text = std::fs::read_to_string(&results_path)
            .unwrap_or_else(|e| panic!("read final results.jsonl: {e}"));
        let final_lines: Vec<_> = final_text.lines().collect();
        assert_eq!(
            final_lines.len(),
            6,
            "final results.jsonl should contain exactly six valid lines"
        );
        for line in final_lines {
            let _: ScoredEdge = serde_json::from_str(line)
                .expect("every final line, including the recovered one, should parse");
        }
    }
}
