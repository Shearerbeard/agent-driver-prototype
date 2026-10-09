//! Leg-2 judge: an external, blinded verdict over the held-out split only.
//!
//! The judge is intentionally isolated from the Jev scorer's self-confidence:
//! it receives the same per-arm evidence windows and worker output, and must
//! return a constrained reply that the runner parses.

use std::collections::HashSet;
use std::env;
use std::time::Duration;

use serde::Serialize;

use crate::arms::{ArmConfig, SelectionArm, assemble_state};
use crate::chat::{ChatMessage, ChatRequest, ChatResponse};
use crate::corpus::{FailureClass, LabelledEdge};
use crate::exhaustion::CompletedKey;
use crate::http_retry::execute_with_retry;
use crate::runner::run_bounded;
use crate::summarizer::FixtureSummarizer;
use crate::verdict::Verdict;

/// Configuration for the Bedrock-runtime judge.
#[derive(Debug, Clone)]
pub struct JudgeConfig {
    pub bearer_token: String,
    pub region: String,
    pub model: String,
    pub base_url: String,
    pub max_tokens: u32,
    pub timeout: Duration,
}

impl JudgeConfig {
    /// Default thinking-model token budget. The probe burned 8 tokens on
    /// reasoning alone; 4096 gives comfortable headroom and is overridable.
    pub const DEFAULT_MAX_TOKENS: u32 = 4096;

    /// Default timeout for a single judge call.
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_mins(2);

    /// Load from environment. Fails loud if the bearer token or region is
    /// missing so the operator knows the judge path is unset before spend.
    pub fn from_env() -> anyhow::Result<Self> {
        let bearer_token = env::var("AWS_BEARER_TOKEN_BEDROCK").map_err(|_| {
            anyhow::anyhow!("AWS_BEARER_TOKEN_BEDROCK is required for live judge runs")
        })?;
        let region = env::var("AWS_REGION")
            .map_err(|_| anyhow::anyhow!("AWS_REGION is required for live judge runs"))?;
        let base_url = env::var("AWS_BEDROCK_BASE_URL")
            .unwrap_or_else(|_| format!("https://bedrock-runtime.{region}.amazonaws.com"));
        let model = env::var("JUDGE_MODEL").unwrap_or_else(|_| "us.moonshotai.kimi-k3".to_owned());
        let max_tokens = env::var("JUDGE_MAX_TOKENS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(Self::DEFAULT_MAX_TOKENS);
        let timeout = env::var("JUDGE_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .map(Duration::from_secs)
            .unwrap_or(Self::DEFAULT_TIMEOUT);
        Ok(Self {
            bearer_token,
            region,
            model,
            base_url,
            max_tokens,
            timeout,
        })
    }
}

/// Live judge client.
#[derive(Debug, Clone)]
pub struct JudgeClient {
    client: reqwest::Client,
    config: JudgeConfig,
}

impl JudgeClient {
    /// Create from config. No HTTP call is made here.
    pub fn new(config: JudgeConfig) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build judge HTTP client: {e}"))?;
        Ok(Self { client, config })
    }

    /// Return the constrained reply format the judge must follow.
    pub fn reply_format() -> &'static str {
        "Reply with exactly one line:\nVERDICT: <PASS|FLAG|INSUFFICIENT> [dimension1,dimension2,...]\n"
    }

    /// Build a blinded judge prompt for one labelled edge under one arm.
    ///
    /// The prompt omits the Jev scorer's confidence, probabilities, and
    /// composite score. It includes the same evidence windows the arm used.
    pub async fn build_prompt(
        &self,
        labelled: &LabelledEdge,
        arm: SelectionArm,
        arm_config: &ArmConfig,
    ) -> anyhow::Result<String> {
        let summarizer = FixtureSummarizer;
        let state = assemble_state(&labelled.edge, arm, arm_config, &summarizer).await?;
        let evidence = state
            .claim_evidence
            .iter()
            .map(|c| format!("- {} (found in {:?})", c.claim, c.found_in))
            .collect::<Vec<_>>()
            .join("\n");
        let context = state.context_provided;
        Ok(format!(
            "You are a blind judge evaluating a worker output against its assigned task.\n\
             Do NOT consider any model confidence or composite score.\n\
             \n\
             Goal: {}\n\
             Task: {}\n\
             Worker output: {}\n\
             Evidence windows:\n{}\n\
             Prior context:\n{}\n\
             {}",
            state.goal,
            state.plan_step.task,
            state.worker_output.result,
            if evidence.is_empty() {
                "(none)".to_owned()
            } else {
                evidence
            },
            if context.is_empty() {
                "(none)".to_owned()
            } else {
                context
            },
            Self::reply_format()
        ))
    }

    /// Parse a constrained judge reply into a verdict.
    pub fn parse_reply(reply: &str) -> anyhow::Result<JudgeVerdict> {
        let line = reply.lines().next().unwrap_or(reply).trim();
        let rest = line
            .strip_prefix("VERDICT:")
            .ok_or_else(|| anyhow::anyhow!("judge reply missing VERDICT: prefix"))?
            .trim();
        let (verdict_str, dims_str) = match rest.split_once(' ') {
            Some((v, d)) => (
                v.trim(),
                d.trim()
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .replace(' ', ""),
            ),
            None => (rest, String::new()),
        };
        let verdict = match verdict_str.to_uppercase().as_str() {
            "PASS" => Verdict::Pass,
            "FLAG" => Verdict::Flag {
                dims: if dims_str.is_empty() {
                    vec!["judge".to_owned()]
                } else {
                    dims_str.split(',').map(|s| s.to_owned()).collect()
                },
            },
            "INSUFFICIENT" => Verdict::InsufficientEvidence,
            other => anyhow::bail!("unknown judge verdict: {other}"),
        };
        Ok(JudgeVerdict { verdict })
    }

    /// Judge a single held-out edge by POSTing to the Bedrock runtime invoke
    /// endpoint. Fails loud on non-2xx and surfaces null/empty content as a
    /// parse failure.
    pub async fn judge_edge(
        &self,
        labelled: &LabelledEdge,
        arm: SelectionArm,
        arm_config: &ArmConfig,
    ) -> anyhow::Result<JudgeVerdict> {
        let prompt = self.build_prompt(labelled, arm, arm_config).await?;
        let request = ChatRequest {
            model: self.config.model.clone(),
            messages: vec![ChatMessage::user(prompt)],
            max_tokens: self.config.max_tokens,
            temperature: 0.0,
            thinking: None,
        };
        let url = format!(
            "{}/model/{}/invoke",
            self.config.base_url.trim_end_matches('/'),
            self.config.model
        );

        let req = self
            .client
            .post(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.config.bearer_token),
            )
            .json(&request)
            .build()
            .map_err(|e| anyhow::anyhow!("judge failed to build request: {e}"))?;
        let response = execute_with_retry(&self.client, req, "judge").await?;

        let chat = response
            .json::<ChatResponse>()
            .await
            .map_err(|e| anyhow::anyhow!("judge response failed to parse: {e}"))?;
        let content = chat
            .first_content()
            .map_err(|e| anyhow::anyhow!("judge response invalid: {e}"))?;
        Self::parse_reply(&content)
    }
}

/// Parsed judge verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JudgeVerdict {
    pub verdict: Verdict,
}

/// Run the judge over a slice of scored edges, verifying the slice is the
/// held-out split before making any calls.
///
/// Board ruling (2026-10-08): the judge must refuse any corpus subset other
/// than the held-out split.
pub async fn judge_held_out<S: std::hash::BuildHasher>(
    client: &JudgeClient,
    edges: &[(&LabelledEdge, SelectionArm)],
    arm_config: &ArmConfig,
    held_out_ids: &HashSet<String, S>,
) -> anyhow::Result<Vec<(String, SelectionArm, JudgeVerdict)>> {
    for (labelled, _arm) in edges {
        if !held_out_ids.contains(&labelled.variant_id) {
            anyhow::bail!(
                "judge refused non-held-out edge {} (board ruling 2026-10-08)",
                labelled.variant_id
            );
        }
    }

    let owned_edges: Vec<_> = edges.iter().map(|(e, arm)| ((*e).clone(), *arm)).collect();
    judge_edges_bounded(client, owned_edges, arm_config, None).await
}

/// Judge a list of held-out edges with bounded concurrency.
///
/// Successfully judged pairs are recorded in `completed` when supplied, so a
/// caller building a resume state can include them.
pub(crate) async fn judge_edges_bounded(
    client: &JudgeClient,
    edges: Vec<(LabelledEdge, SelectionArm)>,
    arm_config: &ArmConfig,
    completed: Option<&mut HashSet<CompletedKey>>,
) -> anyhow::Result<Vec<(String, SelectionArm, JudgeVerdict)>> {
    use crate::runner::run_concurrency;

    let concurrency = run_concurrency();
    let client = client.clone();
    let arm_config = *arm_config;

    let work_items: Vec<_> = edges.into_iter().enumerate().collect();
    let mut judge_slots = Vec::new();

    let (results, first_error) = run_bounded(
        work_items,
        concurrency,
        move |idx: usize, (edge, arm): (LabelledEdge, SelectionArm)| {
            let client = client.clone();
            async move {
                let verdict = client.judge_edge(&edge, arm, &arm_config).await?;
                let key = CompletedKey::new(arm.name(), &edge.variant_id);
                Ok((idx, (edge.variant_id.clone(), arm, verdict, key)))
            }
        },
    )
    .await;

    if let Some(completed) = completed {
        for (variant_id, arm, verdict, key) in results.into_iter().flatten() {
            judge_slots.push((variant_id, arm, verdict));
            completed.insert(key);
        }
    } else {
        for (variant_id, arm, verdict, _key) in results.into_iter().flatten() {
            judge_slots.push((variant_id, arm, verdict));
        }
    }

    let judge_results = judge_slots;

    if let Some(e) = first_error {
        return Err(e);
    }

    Ok(judge_results)
}

/// Return true if a failure class should be sent to the judge. The judge is
/// only invoked on held-out edges; this helper selects which classes within
/// that split are worth an external opinion.
#[allow(
    dead_code,
    reason = "kept for downstream reporting filters; the runner now sends every held-out edge to the judge per board ruling"
)]
pub fn class_is_judgeable(class: FailureClass) -> bool {
    let _ = class;
    true
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use wiremock::matchers::{bearer_token, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::arms::ArmConfig;
    use crate::corpus::{CapturedEdge, FailureClass, LabelledEdge};
    use crate::exhaustion::is_exhaustion;
    use crate::state::{PlanContext, PlanStep, WorkerOutput};

    fn dummy_edge(variant_id: &str, class: FailureClass) -> LabelledEdge {
        LabelledEdge {
            edge: CapturedEdge {
                scenario: "judge-test".to_owned(),
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
                    result: "result".to_owned(),
                    truncated: false,
                },
                tool_calls: Vec::new(),
            },
            class,
            variant_id: variant_id.to_owned(),
        }
    }

    fn test_config(base_url: String) -> JudgeConfig {
        JudgeConfig {
            bearer_token: "test-token".to_owned(),
            region: "us-east-1".to_owned(),
            model: "us.moonshotai.kimi-k3".to_owned(),
            base_url,
            max_tokens: 4096,
            timeout: Duration::from_secs(5),
        }
    }

    fn ok_response(content: &str) -> serde_json::Value {
        serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": content}}]
        })
    }

    fn null_content_response() -> serde_json::Value {
        serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": null}}]
        })
    }

    #[tokio::test]
    async fn happy_path_parses_verdict() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/model/{}/invoke",
                test_config(server.uri()).model
            )))
            .and(bearer_token("test-token"))
            .and(header("content-type", "application/json".to_owned()))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(ok_response("VERDICT: FLAG [evidence_grounding]")),
            )
            .mount(&server)
            .await;

        let config = test_config(server.uri());
        let client = JudgeClient::new(config).unwrap_or_else(|e| panic!("{e}"));
        let edge = dummy_edge("held-1", FailureClass::Drifted);
        let verdict = client
            .judge_edge(&edge, SelectionArm::ClaimIndexed, &ArmConfig::default())
            .await
            .unwrap_or_else(|e| panic!("{e}"));

        assert_eq!(
            verdict.verdict,
            Verdict::Flag {
                dims: vec!["evidence_grounding".to_owned()],
            }
        );
    }

    #[tokio::test]
    async fn non_2xx_fails_loud() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/model/{}/invoke",
                test_config(server.uri()).model
            )))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let config = test_config(server.uri());
        let client = JudgeClient::new(config).unwrap_or_else(|e| panic!("{e}"));
        let edge = dummy_edge("held-1", FailureClass::Drifted);
        let err = match client
            .judge_edge(&edge, SelectionArm::ClaimIndexed, &ArmConfig::default())
            .await
        {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected error"),
        };

        assert!(err.contains("HTTP 500"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn status_429_is_classified_as_exhaustion() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/model/{}/invoke",
                test_config(server.uri()).model
            )))
            .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
            .mount(&server)
            .await;

        let config = test_config(server.uri());
        let client = JudgeClient::new(config).unwrap_or_else(|e| panic!("{e}"));
        let edge = dummy_edge("held-1", FailureClass::Drifted);
        let err = match client
            .judge_edge(&edge, SelectionArm::ClaimIndexed, &ArmConfig::default())
            .await
        {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected error"),
        };

        assert!(err.contains("exhausted: 429"), "unexpected error: {err}");
        assert!(is_exhaustion(&anyhow::anyhow!("{err}")));
    }

    #[tokio::test]
    async fn null_content_surfaces_as_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/model/{}/invoke",
                test_config(server.uri()).model
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(null_content_response()))
            .mount(&server)
            .await;

        let config = test_config(server.uri());
        let client = JudgeClient::new(config).unwrap_or_else(|e| panic!("{e}"));
        let edge = dummy_edge("held-1", FailureClass::Drifted);
        let err = match client
            .judge_edge(&edge, SelectionArm::ClaimIndexed, &ArmConfig::default())
            .await
        {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected error"),
        };

        assert!(err.contains("content is null"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn held_out_guard_refuses_calibration_edge() {
        let server = MockServer::start().await;
        let config = test_config(server.uri());
        let client = JudgeClient::new(config).unwrap_or_else(|e| panic!("{e}"));
        let held_out_ids: HashSet<String> = ["held-1".to_owned()].into_iter().collect();
        let cal_edge = dummy_edge("cal-1", FailureClass::Clean);
        let edges = vec![(&cal_edge, SelectionArm::ClaimIndexed)];

        let result = judge_held_out(&client, &edges, &ArmConfig::default(), &held_out_ids).await;

        assert!(result.is_err(), "judge must refuse calibration edges");
        let err = match result {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected refusal"),
        };
        assert!(err.contains("refused"), "error should explain refusal");
    }
}
