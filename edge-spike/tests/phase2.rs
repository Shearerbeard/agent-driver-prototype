//! Phase 2 wiring tests: judge guardrails, dry-run plan, and exhaustion
//! bookkeeping. These tests make no network calls.

use std::collections::HashSet;

use edge_spike::arms::{ArmConfig, SelectionArm};
use edge_spike::corpus::{CapturedEdge, Corpus, FailureClass, LabelledEdge, ToolCall};
use edge_spike::exhaustion::{CompletedKey, ResumeState, is_exhaustion};
use edge_spike::judge::{JudgeClient, JudgeConfig, judge_held_out};
use edge_spike::runner::{RunnerConfig, plan_run, run_corpus};
use edge_spike::state::{PlanContext, PlanStep, TaskRef, WorkerOutput};
use std::time::Duration;
use wiremock::matchers::{bearer_token, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn dummy_edge(variant_id: &str, class: FailureClass) -> LabelledEdge {
    LabelledEdge {
        edge: CapturedEdge {
            scenario: "phase2".to_owned(),
            task_id: 1,
            worker_id: "w1".to_owned(),
            goal: "test goal".to_owned(),
            plan_step: PlanStep {
                task: "test task".to_owned(),
                worker_role: "worker".to_owned(),
                depends_on: Vec::new(),
            },
            plan_context: PlanContext {
                consumers: vec![TaskRef {
                    task: "consumer".to_owned(),
                }],
                siblings: vec![TaskRef {
                    task: "sibling".to_owned(),
                }],
            },
            context_provided: "prior context".to_owned(),
            worker_output: WorkerOutput {
                summary: "summary".to_owned(),
                result: "result".to_owned(),
                truncated: false,
            },
            tool_calls: vec![ToolCall {
                tool_call_id: "tc1".to_owned(),
                tool_name: "lookup".to_owned(),
                arguments: serde_json::json!({"q": "x"}),
                result: "capture data".to_owned(),
                success: true,
            }],
        },
        class,
        variant_id: variant_id.to_owned(),
    }
}

fn minimal_corpus() -> Corpus {
    Corpus {
        calibration: vec![dummy_edge("cal-1", FailureClass::Clean)],
        held_out: vec![dummy_edge("held-1", FailureClass::Drifted)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_zero_network() {
        let corpus = minimal_corpus();
        let config = RunnerConfig::default();
        let plan = plan_run(&corpus, &config);

        // Replay mode does not schedule judge calls.
        assert_eq!(plan.judge_calls, 0);

        // One edge x three arms.
        let claim_indexed = plan
            .per_arm
            .get("claim_indexed")
            .unwrap_or_else(|| panic!("claim_indexed arm missing from plan"));
        assert_eq!(claim_indexed.edges, 2);
        assert_eq!(claim_indexed.calls, 2);
        assert!(claim_indexed.estimated_input_tokens > 0);
        assert!(claim_indexed.estimated_output_tokens > 0);

        let formatted = plan.format();
        assert!(formatted.contains("# Dry-run plan"));
        assert!(formatted.contains("Network calls: 0 (dry run)"));
        assert_eq!(
            plan.estimated_input_tokens,
            plan.per_arm
                .values()
                .map(|p| p.estimated_input_tokens)
                .sum::<u64>()
        );
    }

    #[tokio::test]
    async fn judge_refuses_calibration_edge() {
        let corpus = minimal_corpus();
        let held_out_ids: HashSet<String> = corpus
            .held_out
            .iter()
            .map(|e| e.variant_id.clone())
            .collect();
        let client = JudgeClient::new(JudgeConfig {
            bearer_token: "test-token".to_owned(),
            region: "us-east-1".to_owned(),
            model: "us.moonshotai.kimi-k3".to_owned(),
            base_url: "https://bedrock-runtime.us-east-1.amazonaws.com".to_owned(),
            max_tokens: 4096,
            timeout: Duration::from_secs(5),
        })
        .unwrap_or_else(|e| panic!("judge client: {e}"));
        let edges = vec![(&corpus.calibration[0], SelectionArm::ClaimIndexed)];

        let result = judge_held_out(&client, &edges, &ArmConfig::default(), &held_out_ids).await;

        assert!(result.is_err(), "judge must refuse non-held-out edges");
        let err = match result {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected refusal"),
        };
        assert!(
            err.contains("refused"),
            "error should explain refusal: {err}"
        );
        assert!(
            err.contains("cal-1"),
            "error should name the rejected variant: {err}"
        );
    }

    #[tokio::test]
    async fn judge_accepts_held_out_edge() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/model/us.moonshotai.kimi-k3/invoke"))
            .and(bearer_token("test-token"))
            .and(header("content-type", "application/json".to_owned()))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"role": "assistant", "content": "VERDICT: PASS"}}]
            })))
            .mount(&server)
            .await;

        let corpus = minimal_corpus();
        let held_out_ids: HashSet<String> = corpus
            .held_out
            .iter()
            .map(|e| e.variant_id.clone())
            .collect();
        let client = JudgeClient::new(JudgeConfig {
            bearer_token: "test-token".to_owned(),
            region: "us-east-1".to_owned(),
            model: "us.moonshotai.kimi-k3".to_owned(),
            base_url: server.uri(),
            max_tokens: 4096,
            timeout: Duration::from_secs(5),
        })
        .unwrap_or_else(|e| panic!("judge client: {e}"));
        let edges = vec![(&corpus.held_out[0], SelectionArm::ClaimIndexed)];

        let result = judge_held_out(&client, &edges, &ArmConfig::default(), &held_out_ids)
            .await
            .unwrap_or_else(|e| panic!("judge should accept held-out edge: {e}"));

        assert_eq!(result.len(), 1);
    }

    #[test]
    fn exhaustion_detection() {
        assert!(is_exhaustion(&anyhow::anyhow!("429 rate limited")));
        assert!(is_exhaustion(&anyhow::anyhow!("quota exceeded")));
        assert!(is_exhaustion(&anyhow::anyhow!("service exhausted")));
        assert!(!is_exhaustion(&anyhow::anyhow!("connection reset by peer")));
        assert!(!is_exhaustion(&anyhow::anyhow!("malformed response")));
    }

    #[test]
    fn resume_state_records_completed_and_error() {
        let completed = vec![
            CompletedKey::new("claim_indexed", "v1"),
            CompletedKey::new("head_truncation", "v1"),
        ];
        let resume = ResumeState::exhausted("jev_scorer", completed.clone(), "429 rate limited");

        assert_eq!(resume.completed, completed);
        assert!(resume.is_completed("claim_indexed", "v1"));
        assert!(!resume.is_completed("flash_summarization", "v1"));
        assert!(resume.error.contains("jev_scorer"));
        assert!(resume.error.contains("429 rate limited"));
    }

    #[test]
    fn resume_state_serializes_to_artifact_dir() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("temp dir: {e}"));
        let resume = ResumeState::exhausted(
            "judge",
            vec![CompletedKey::new("claim_indexed", "v1")],
            "quota exceeded",
        );
        resume
            .write(dir.path())
            .unwrap_or_else(|e| panic!("write resume: {e}"));

        let path = dir.path().join("resume.json");
        let json = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read resume: {e}"));
        assert!(json.contains("quota exceeded"));
        assert!(json.contains("claim_indexed"));
        assert!(json.contains("v1"));
    }

    #[tokio::test]
    async fn resumed_run_skips_completed_arm_variant_pairs() {
        let corpus = minimal_corpus();
        let config = RunnerConfig::default();
        let resume = ResumeState::exhausted(
            "jev_scorer",
            vec![CompletedKey::new("claim_indexed", "cal-1")],
            "test",
        );

        let result = run_corpus(&corpus, &config, Some(&resume))
            .await
            .unwrap_or_else(|e| panic!("run_corpus failed: {e}"));

        let expected = corpus.total() * SelectionArm::all().len() - 1;
        assert_eq!(result.scored.len(), expected);

        let keys: HashSet<(String, String)> = result
            .scored
            .iter()
            .map(|s| (s.arm.name().to_owned(), s.variant_id.clone()))
            .collect();
        assert_eq!(
            keys.len(),
            result.scored.len(),
            "scored pairs must be unique"
        );
        assert!(
            !keys.contains(&("claim_indexed".to_owned(), "cal-1".to_owned())),
            "resumed pair must be skipped"
        );
    }
}
