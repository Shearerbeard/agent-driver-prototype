//! Phase 2 wiring tests: judge guardrails, dry-run plan, and exhaustion
//! bookkeeping. These tests make no network calls.

use std::collections::HashSet;

use edge_spike::arms::{ArmConfig, SelectionArm};
use edge_spike::corpus::{
    CapturedEdge, Corpus, FailureClass, LabelledEdge, ToolCall, default_corpus,
};
use edge_spike::exhaustion::{CompletedKey, ResumeState, is_exhaustion};
use edge_spike::judge::{JudgeClient, JudgeConfig, judge_held_out};
use edge_spike::runner::{RunPlan, RunSelector, RunnerConfig, plan_run, run_corpus};
use edge_spike::state::{PlanContext, PlanStep, TaskRef, WorkerOutput};
use std::time::Duration;
use wiremock::matchers::{bearer_token, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn dummy_edge(variant_id: &str, class: FailureClass) -> LabelledEdge {
    LabelledEdge {
        edge: CapturedEdge {
            scenario: "phase2".to_owned(),
            iteration: 0,
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
            confidence: None,
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use edge_spike::client::JevEndpoint;
    use edge_spike::runner::RunnerMode;
    use wiremock::{Request, Respond};

    #[test]
    fn dry_run_zero_network() {
        let corpus = minimal_corpus();
        let config = RunnerConfig::default();
        let plan = plan_run(&corpus, &config).unwrap_or_else(|e| panic!("plan_run: {e}"));

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

    fn live_config(selector: RunSelector) -> RunnerConfig {
        RunnerConfig {
            mode: RunnerMode::Live,
            selector,
            judge: true,
            ..RunnerConfig::default()
        }
    }

    fn assert_edges_per_arm(plan: &RunPlan, expected: usize) {
        for arm in SelectionArm::all() {
            let arm_plan = plan
                .per_arm
                .get(arm.name())
                .unwrap_or_else(|| panic!("{} arm missing from plan", arm.name()));
            assert_eq!(arm_plan.edges, expected, "{} edges", arm.name());
            assert_eq!(arm_plan.calls, expected, "{} calls", arm.name());
        }
    }

    #[test]
    fn dry_run_smoke_selector_counts_one_calibration_edge() {
        let corpus = default_corpus().unwrap_or_else(|e| panic!("default corpus: {e}"));
        let plan = plan_run(&corpus, &live_config(RunSelector::Smoke))
            .unwrap_or_else(|e| panic!("plan_run: {e}"));

        assert_edges_per_arm(&plan, 1);
        assert_eq!(plan.judge_calls, 0, "smoke edge is calibration");
    }

    #[test]
    fn dry_run_slice_selector_counts_calibration_slice() {
        let corpus = default_corpus().unwrap_or_else(|e| panic!("default corpus: {e}"));
        let plan = plan_run(&corpus, &live_config(RunSelector::Slice(5)))
            .unwrap_or_else(|e| panic!("plan_run: {e}"));

        assert_edges_per_arm(&plan, 5);
        assert_eq!(plan.judge_calls, 0, "slice draws from calibration");
    }

    #[test]
    fn dry_run_all_selector_matches_corpus_totals() {
        let corpus = default_corpus().unwrap_or_else(|e| panic!("default corpus: {e}"));
        let plan = plan_run(&corpus, &live_config(RunSelector::All))
            .unwrap_or_else(|e| panic!("plan_run: {e}"));

        assert_edges_per_arm(&plan, corpus.total());
        assert_eq!(
            plan.judge_calls,
            corpus.held_out.len() * SelectionArm::all().len(),
            "every held-out variant is judged once per arm"
        );
    }

    #[test]
    fn dry_run_edge_selector_judges_selected_held_out_edge() {
        let corpus = default_corpus().unwrap_or_else(|e| panic!("default corpus: {e}"));
        let held_out_id = corpus.held_out[0].variant_id.clone();
        let plan = plan_run(&corpus, &live_config(RunSelector::Edge(held_out_id)))
            .unwrap_or_else(|e| panic!("plan_run: {e}"));

        assert_edges_per_arm(&plan, 1);
        assert_eq!(
            plan.judge_calls,
            SelectionArm::all().len(),
            "a selected held-out edge is judged once per arm"
        );
    }

    #[test]
    fn dry_run_default_judge_off_reports_zero() {
        let corpus = default_corpus().unwrap_or_else(|e| panic!("default corpus: {e}"));
        let config = RunnerConfig {
            mode: RunnerMode::Live,
            selector: RunSelector::All,
            ..RunnerConfig::default()
        };

        let plan = plan_run(&corpus, &config).unwrap_or_else(|e| panic!("plan_run: {e}"));

        assert_eq!(plan.judge_calls, 0, "judge must be off unless selected");
        assert!(
            plan.format().contains("Judge calls: 0"),
            "preview must show zero judge calls when off: {}",
            plan.format()
        );
    }

    #[tokio::test]
    async fn live_default_judge_off_makes_no_judge_calls_with_env_set() {
        let _guard = ENV_LOCK.lock().await;

        let jev_server = MockServer::start().await;
        let sum_server = MockServer::start().await;
        let judge_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .respond_with(ResponseTemplate::new(200).set_body_json(jev_ok_response()))
            .mount(&jev_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(summary_ok_response()))
            .mount(&sum_server)
            .await;

        set_live_env(&jev_server.uri(), &sum_server.uri(), &judge_server.uri());

        let corpus = held_out_corpus(1);
        let config = RunnerConfig {
            mode: RunnerMode::Live,
            endpoint: JevEndpoint::Cloud,
            ..Default::default()
        };
        let result = run_corpus(&corpus, &config, None)
            .await
            .unwrap_or_else(|e| panic!("run_corpus failed: {e}"));

        clear_live_env();

        assert!(
            result.judge_results.is_empty(),
            "judge off must produce zero judge results"
        );
        let received = judge_server.received_requests().await.unwrap_or_default();
        assert!(
            received.is_empty(),
            "judge endpoint must not be called when judge is off: {received:?}"
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
            max_tokens: 8192,
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
            max_tokens: 8192,
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
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("temp dir: {e}"));

        let full = run_corpus(&corpus, &config, Some(dir.path()))
            .await
            .unwrap_or_else(|e| panic!("run_corpus failed: {e}"));
        assert_eq!(
            full.scored.len(),
            corpus.total() * SelectionArm::all().len()
        );

        // Simulate a partial resume by keeping only the claim_indexed cal-1 record.
        let results_path = dir.path().join(edge_spike::runner::RESULTS_JSONL);
        let lines: Vec<_> = std::fs::read_to_string(&results_path)
            .unwrap_or_else(|e| panic!("read results.jsonl: {e}"))
            .lines()
            .map(|s| s.to_owned())
            .collect();
        let keep: Vec<_> = lines
            .into_iter()
            .filter(|line| {
                let edge: edge_spike::runner::ScoredEdge =
                    serde_json::from_str(line).expect("valid scored edge");
                edge.variant_id == "cal-1" && edge.arm.name() == "claim_indexed"
            })
            .collect();
        std::fs::write(&results_path, keep.join("\n") + "\n")
            .unwrap_or_else(|e| panic!("write partial results.jsonl: {e}"));

        let result = run_corpus(&corpus, &config, Some(dir.path()))
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

    #[tokio::test]
    async fn corpus_scoring_preserves_deterministic_order() {
        let corpus = minimal_corpus();
        let config = RunnerConfig::default();

        let result = run_corpus(&corpus, &config, None)
            .await
            .unwrap_or_else(|e| panic!("run_corpus failed: {e}"));

        let expected_order: Vec<_> = corpus
            .calibration
            .iter()
            .chain(corpus.held_out.iter())
            .flat_map(|edge| {
                SelectionArm::all()
                    .iter()
                    .map(move |arm| (edge.variant_id.clone(), *arm))
            })
            .collect();

        let actual_order: Vec<_> = result
            .scored
            .iter()
            .map(|s| (s.variant_id.clone(), s.arm))
            .collect();

        assert_eq!(actual_order, expected_order);
    }

    #[tokio::test]
    async fn judge_drains_in_flight_on_exhaustion() {
        let _guard = ENV_LOCK.lock().await;

        use std::sync::atomic::{AtomicUsize, Ordering};
        use wiremock::{Request, Respond, ResponseTemplate};

        struct ExhaustOnFirstRequest {
            count: AtomicUsize,
        }

        impl Respond for ExhaustOnFirstRequest {
            fn respond(&self, _request: &Request) -> ResponseTemplate {
                let n = self.count.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    ResponseTemplate::new(429).set_body_string("rate limited")
                } else {
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({
                            "choices": [{"message": {"role": "assistant", "content": "VERDICT: PASS"}}]
                        }))
                        .set_delay(Duration::from_millis(100))
                }
            }
        }

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/model/us.moonshotai.kimi-k3/invoke"))
            .respond_with(ExhaustOnFirstRequest {
                count: AtomicUsize::new(0),
            })
            .expect(4)
            .mount(&server)
            .await;

        let client = JudgeClient::new(JudgeConfig {
            bearer_token: "test-token".to_owned(),
            region: "us-east-1".to_owned(),
            model: "us.moonshotai.kimi-k3".to_owned(),
            base_url: server.uri(),
            max_tokens: 8192,
            timeout: Duration::from_secs(5),
        })
        .unwrap_or_else(|e| panic!("judge client: {e}"));

        let edges: Vec<_> = (0..4)
            .map(|i| {
                (
                    dummy_edge(&format!("held-{i}"), FailureClass::Drifted),
                    SelectionArm::ClaimIndexed,
                )
            })
            .collect();
        let held_out_ids: HashSet<String> =
            edges.iter().map(|(e, _)| e.variant_id.clone()).collect();
        let edge_refs: Vec<_> = edges.iter().map(|(e, arm)| (e, *arm)).collect();

        let result =
            judge_held_out(&client, &edge_refs, &ArmConfig::default(), &held_out_ids).await;

        assert!(result.is_err(), "judge must surface exhaustion");
        let err = result.unwrap_err().to_string();
        assert!(
            is_exhaustion(&anyhow::anyhow!("{err}")),
            "error should be exhaustion: {err}"
        );
    }

    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn held_out_corpus(held_out_count: usize) -> Corpus {
        Corpus {
            calibration: vec![dummy_edge("cal-1", FailureClass::Clean)],
            held_out: (0..held_out_count)
                .map(|i| dummy_edge(&format!("held-{i}"), FailureClass::Drifted))
                .collect(),
        }
    }

    fn jev_ok_response() -> serde_json::Value {
        serde_json::json!({
            "model": "jev-1.13.0-replay",
            "answers": {
                "evidence_presence": {"type": "noul", "noul": 0.95},
                "task_fidelity": {
                    "type": "score",
                    "score": 4.0,
                    "legend": {"0": "none", "1": "poor", "2": "fair", "3": "good", "4": "excellent"},
                    "probabilities": {"0": 0.1, "1": 0.2, "2": 0.4, "3": 0.2, "4": 0.1},
                    "confidence": 0.7
                },
                "goal_alignment": {
                    "type": "score",
                    "score": 2.0,
                    "legend": {"0": "misaligned", "1": "neutral", "2": "aligned"},
                    "probabilities": {"0": 0.2, "1": 0.6, "2": 0.2},
                    "confidence": 0.7
                },
                "evidence_grounding": {
                    "type": "score",
                    "score": 3.0,
                    "legend": {"0": "unsupported", "1": "partially", "2": "mostly", "3": "supported"},
                    "probabilities": {"0": 0.1, "1": 0.2, "2": 0.5, "3": 0.2},
                    "confidence": 0.7
                },
                "handoff_integrity": {"type": "noul", "noul": 0.9},
                "scope_discipline": {"type": "noul", "noul": 0.9}
            },
            "usage": {"input_tokens": 2048, "output_tokens": 128}
        })
    }

    fn summary_ok_response() -> serde_json::Value {
        serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": "short summary"}}]
        })
    }

    fn judge_ok_response() -> serde_json::Value {
        serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": "VERDICT: PASS"}}]
        })
    }

    fn null_content_response() -> serde_json::Value {
        serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": null}}]
        })
    }

    struct FirstNThenNullResponder {
        successes: usize,
        attempt: AtomicUsize,
    }

    impl Respond for FirstNThenNullResponder {
        fn respond(&self, _request: &Request) -> ResponseTemplate {
            let n = self.attempt.fetch_add(1, Ordering::SeqCst);
            if n < self.successes {
                ResponseTemplate::new(200).set_body_json(judge_ok_response())
            } else {
                ResponseTemplate::new(200).set_body_json(null_content_response())
            }
        }
    }

    fn set_live_env(jev_uri: &str, sum_uri: &str, judge_uri: &str) {
        unsafe {
            std::env::set_var("RUN_CONCURRENCY", "1");
            std::env::set_var("TYPESAFE_BASE_URL", jev_uri);
            std::env::set_var("TYPESAFE_API_KEY", "test-key");
            std::env::set_var("TYPESAFE_MODEL", "jev-1.13.0");
            std::env::set_var("SUMMARIZER_BASE_URL", sum_uri);
            std::env::set_var("SUMMARIZER_API_KEY", "test-key");
            std::env::set_var("SUMMARIZER_MODEL", "test-model");
            std::env::set_var("AWS_BEARER_TOKEN_BEDROCK", "test-token");
            std::env::set_var("AWS_REGION", "us-east-1");
            std::env::set_var("AWS_BEDROCK_BASE_URL", judge_uri);
            std::env::set_var("JUDGE_MODEL", "us.moonshotai.kimi-k3");
        }
    }

    fn clear_live_env() {
        unsafe {
            std::env::remove_var("RUN_CONCURRENCY");
            std::env::remove_var("TYPESAFE_BASE_URL");
            std::env::remove_var("TYPESAFE_API_KEY");
            std::env::remove_var("TYPESAFE_MODEL");
            std::env::remove_var("SUMMARIZER_BASE_URL");
            std::env::remove_var("SUMMARIZER_API_KEY");
            std::env::remove_var("SUMMARIZER_MODEL");
            std::env::remove_var("AWS_BEARER_TOKEN_BEDROCK");
            std::env::remove_var("AWS_REGION");
            std::env::remove_var("AWS_BEDROCK_BASE_URL");
            std::env::remove_var("JUDGE_MODEL");
        }
    }

    #[tokio::test]
    async fn judge_halt_preserves_completed_judge_results_and_resume_keys() {
        let _guard = ENV_LOCK.lock().await;

        let jev_server = MockServer::start().await;
        let sum_server = MockServer::start().await;
        let judge_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .respond_with(ResponseTemplate::new(200).set_body_json(jev_ok_response()))
            .expect(12)
            .mount(&jev_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(summary_ok_response()))
            .mount(&sum_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/model/us.moonshotai.kimi-k3/invoke"))
            .respond_with(FirstNThenNullResponder {
                successes: 2,
                attempt: AtomicUsize::new(0),
            })
            .expect(5)
            .mount(&judge_server)
            .await;

        set_live_env(&jev_server.uri(), &sum_server.uri(), &judge_server.uri());

        let corpus = held_out_corpus(3);
        let config = RunnerConfig {
            mode: RunnerMode::Live,
            endpoint: JevEndpoint::Cloud,
            judge: true,
            ..Default::default()
        };
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("temp dir: {e}"));
        let result = run_corpus(&corpus, &config, Some(dir.path()))
            .await
            .unwrap_or_else(|e| panic!("run_corpus failed: {e}"));

        clear_live_env();

        assert_eq!(result.scored.len(), 12, "all scoring pairs should complete");
        assert_eq!(
            result.judge_results.len(),
            2,
            "completed judge pairs before halt must be preserved"
        );

        let resume = result.resume.expect("expected resume state");
        assert!(
            is_exhaustion(&anyhow::anyhow!("{}", resume.error)),
            "expected exhaustion: {}",
            resume.error
        );
        assert!(
            resume.error.contains("judge"),
            "error should name judge: {}",
            resume.error
        );
        assert_eq!(
            resume.completed.len(),
            14,
            "resume should contain all scoring keys plus the two judge keys"
        );
        assert!(resume.is_judge_completed("head_truncation", "held-0"));
        assert!(resume.is_judge_completed("claim_indexed", "held-0"));
        assert!(!resume.is_judge_completed("flash_summarization", "held-0"));

        let judge_path = dir.path().join(edge_spike::runner::JUDGE_RESULTS_JSONL);
        let persisted = std::fs::read_to_string(&judge_path)
            .unwrap_or_else(|e| panic!("read judge_results.jsonl: {e}"));
        assert_eq!(
            persisted.lines().count(),
            2,
            "two judge records should be durably persisted"
        );
    }

    #[tokio::test]
    async fn resumed_judge_run_skips_completed_pairs() {
        let _guard = ENV_LOCK.lock().await;

        let jev_server = MockServer::start().await;
        let sum_server = MockServer::start().await;
        let judge_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .respond_with(ResponseTemplate::new(200).set_body_json(jev_ok_response()))
            .expect(9)
            .mount(&jev_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(summary_ok_response()))
            .mount(&sum_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/model/us.moonshotai.kimi-k3/invoke"))
            .respond_with(ResponseTemplate::new(200).set_body_json(judge_ok_response()))
            .mount(&judge_server)
            .await;

        set_live_env(&jev_server.uri(), &sum_server.uri(), &judge_server.uri());

        let corpus = held_out_corpus(2);
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("temp dir: {e}"));
        let config = RunnerConfig {
            mode: RunnerMode::Live,
            endpoint: JevEndpoint::Cloud,
            judge: true,
            ..Default::default()
        };

        // Seed the output directory with a full scoring log and one completed judge pair.
        let full = run_corpus(&corpus, &config, Some(dir.path()))
            .await
            .unwrap_or_else(|e| panic!("seed run failed: {e}"));
        assert_eq!(full.scored.len(), 9);
        assert_eq!(full.judge_results.len(), 6);

        let judge_path = dir.path().join(edge_spike::runner::JUDGE_RESULTS_JSONL);
        let lines: Vec<_> = std::fs::read_to_string(&judge_path)
            .unwrap_or_else(|e| panic!("read judge_results.jsonl: {e}"))
            .lines()
            .map(|s| s.to_owned())
            .collect();
        let keep: Vec<_> = lines
            .into_iter()
            .filter(|line| {
                let record: edge_spike::runner::JudgeResultRecord =
                    serde_json::from_str(line).expect("valid judge record");
                record.variant_id == "held-0" && record.arm.name() == "head_truncation"
            })
            .collect();
        std::fs::write(&judge_path, keep.join("\n") + "\n")
            .unwrap_or_else(|e| panic!("write partial judge_results.jsonl: {e}"));

        let result = run_corpus(&corpus, &config, Some(dir.path()))
            .await
            .unwrap_or_else(|e| panic!("run_corpus failed: {e}"));

        clear_live_env();

        assert!(result.resume.is_none(), "full run should not halt");
        assert_eq!(
            result.scored.len(),
            0,
            "scoring should be skipped because results.jsonl is complete"
        );
        assert_eq!(
            result.judge_results.len(),
            5,
            "judge should skip one completed pair and judge the rest"
        );

        let judged: HashSet<(String, String)> = result
            .judge_results
            .iter()
            .map(|(variant_id, arm, _)| (arm.name().to_owned(), variant_id.clone()))
            .collect();
        assert!(
            !judged.contains(&("head_truncation".to_owned(), "held-0".to_owned())),
            "completed judge pair must be skipped"
        );
        assert!(judged.contains(&("claim_indexed".to_owned(), "held-0".to_owned())));
        assert!(judged.contains(&("flash_summarization".to_owned(), "held-1".to_owned())));
    }
}
