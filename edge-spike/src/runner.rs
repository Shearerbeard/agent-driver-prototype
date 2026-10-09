//! Replay/live runner: corpus x arms scored through replay fixtures or a real
//! Jev client, plus optional Leg-2 judge over the held-out split.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use jev_driver::client::Transport;
use jev_driver::prelude::*;
use jev_driver::wire::{WireAnswer, WireResponse, WireUsage};
use serde::{Deserialize, Serialize};

use crate::arms::{ArmConfig, SelectionArm, assemble_state};
use crate::claim::{extract_claims, resolve_claims};
use crate::client::{JevEndpoint, build_jev_client, resolve_model_id};
use crate::corpus::{Corpus, FailureClass, LabelledEdge};
use crate::exhaustion::{CompletedKey, ResumeState, is_exhaustion};
use crate::judge::{JudgeClient, JudgeConfig, JudgeVerdict, judge_edges_bounded};
use crate::rubric::{EdgeRubric, EdgeRubricAnswers};
use crate::state::EdgeState;
use crate::summarizer::{FixtureSummarizer, FlashSummarizer, LiveSummarizer, SummarizerConfig};
use crate::verdict::{
    TokenUsage, TruncationFlags, VerdictArtifact, Weights, hash_state, resolve_verdict,
    score_composite,
};

/// Scripted Jev response for one edge (in-memory only; not serialized).
#[derive(Debug, Clone)]
pub struct ScriptedResponse {
    pub model: String,
    pub answers: BTreeMap<String, WireAnswer>,
    pub usage: WireUsage,
}

impl ScriptedResponse {
    /// Legacy deterministic fixture for tests that build their own transport.
    ///
    /// New replay code uses [`class_aware_response`] so offline variants
    /// exercise the scoring pipeline with class-differentiated answers.
    #[must_use]
    pub fn fixture() -> Self {
        default_response()
    }
}

/// Local scripted transport so the spike never touches the network.
pub struct ScriptedTransport {
    scripted: std::sync::Mutex<VecDeque<jev_driver::prelude::JevResult<WireResponse>>>,
}

impl ScriptedTransport {
    /// Create from a queue of responses.
    #[must_use]
    pub fn new(scripted: Vec<jev_driver::prelude::JevResult<WireResponse>>) -> Self {
        Self {
            scripted: std::sync::Mutex::new(scripted.into_iter().collect()),
        }
    }
}

#[async_trait]
impl Transport for ScriptedTransport {
    async fn post_systemone(&self, _body: String) -> jev_driver::prelude::JevResult<WireResponse> {
        let mut queue = self
            .scripted
            .lock()
            .map_err(|_| JevError::Transport("scripted transport poisoned".to_owned()))?;
        queue.pop_front().unwrap_or_else(|| {
            Err(JevError::Transport(
                "scripted transport exhausted".to_owned(),
            ))
        })
    }
}

/// How the runner obtains scorer answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerMode {
    /// Deterministic class-aware fixtures (default).
    Replay,
    /// Real Jev client over HTTP.
    Live,
}

/// Runner configuration.
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    pub weights: Weights,
    pub arm_config: ArmConfig,
    pub model_id: String,
    pub mode: RunnerMode,
    pub endpoint: JevEndpoint,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            weights: Weights::default(),
            arm_config: ArmConfig::default(),
            model_id: "jev-1.13.0".to_owned(),
            mode: RunnerMode::Replay,
            endpoint: JevEndpoint::Cloud,
        }
    }
}

/// Result of scoring one labelled edge under one arm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredEdge {
    pub variant_id: String,
    pub class: FailureClass,
    pub arm: SelectionArm,
    pub artifact: VerdictArtifact,
}

/// Result of a full run, possibly halted early by endpoint exhaustion.
#[derive(Debug, Clone)]
pub struct LiveRunResult {
    pub scored: Vec<ScoredEdge>,
    pub judge_results: Vec<(String, SelectionArm, JudgeVerdict)>,
    pub resume: Option<ResumeState>,
}

/// Per-arm plan for dry-run reporting.
#[derive(Debug, Clone, Default)]
pub struct ArmPlan {
    pub edges: usize,
    pub calls: usize,
    pub estimated_input_tokens: u64,
    pub estimated_output_tokens: u64,
}

/// Overall dry-run plan.
#[derive(Debug, Clone, Default)]
pub struct RunPlan {
    pub per_arm: BTreeMap<String, ArmPlan>,
    pub judge_calls: usize,
    pub estimated_input_tokens: u64,
    pub estimated_output_tokens: u64,
}

impl RunPlan {
    /// Human-readable plan summary.
    pub fn format(&self) -> String {
        let mut lines = vec![
            "# Dry-run plan".to_owned(),
            String::new(),
            "| arm | edges | calls | est_input_tokens | est_output_tokens |".to_owned(),
            "| --- | --- | --- | --- | --- |".to_owned(),
        ];
        for (arm, plan) in &self.per_arm {
            lines.push(format!(
                "| {arm} | {} | {} | {} | {} |",
                plan.edges, plan.calls, plan.estimated_input_tokens, plan.estimated_output_tokens
            ));
        }
        lines.extend([
            String::new(),
            format!("- Judge calls: {}", self.judge_calls),
            format!(
                "- Total estimated input tokens: {}",
                self.estimated_input_tokens
            ),
            format!(
                "- Total estimated output tokens: {}",
                self.estimated_output_tokens
            ),
            "- Network calls: 0 (dry run)".to_owned(),
        ]);
        lines.join("\n")
    }
}

/// Build a dry-run plan without constructing any live clients.
pub fn plan_run(corpus: &Corpus, config: &RunnerConfig) -> RunPlan {
    let mut plan = RunPlan::default();
    let input_per_call = 2048u64;
    let output_per_call = 128u64;

    for _edge in corpus.calibration.iter().chain(corpus.held_out.iter()) {
        for arm in SelectionArm::all() {
            let entry = plan.per_arm.entry(arm.name().to_owned()).or_default();
            entry.edges += 1;
            entry.calls += 1;
            entry.estimated_input_tokens += input_per_call;
            entry.estimated_output_tokens += output_per_call;
            plan.estimated_input_tokens += input_per_call;
            plan.estimated_output_tokens += output_per_call;
        }
    }

    if config.mode == RunnerMode::Live {
        // Board ruling (2026-10-08): judge runs on every held-out variant,
        // once per arm. Held-out variants = 407; arms = 3; judge calls = 1221.
        plan.judge_calls = corpus
            .held_out
            .len()
            .saturating_mul(SelectionArm::all().len());
    }

    plan
}

/// Read the optional `RUN_CONCURRENCY` environment override.
///
/// Defaults to 4; values below 1 are clamped to 1 to avoid deadlock.
pub(crate) fn run_concurrency() -> usize {
    std::env::var("RUN_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4)
        .max(1)
}

/// Execute a bounded number of indexed work items concurrently, preserving
/// deterministic output order.
///
/// Scheduling stops as soon as any task returns an error, but tasks already
/// in flight are allowed to finish. Results are collected into pre-allocated
/// slots by their original index.
pub(crate) async fn run_bounded<I, T, F, Fut>(
    items: Vec<(usize, I)>,
    concurrency: usize,
    worker: F,
) -> (Vec<Option<T>>, Option<anyhow::Error>)
where
    I: Send + 'static,
    T: Send + 'static,
    F: Fn(usize, I) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = anyhow::Result<(usize, T)>> + Send + 'static,
{
    let concurrency = concurrency.max(1);
    let total = items.len();
    let mut results: Vec<Option<T>> = (0..total).map(|_| None).collect();
    let mut tasks = tokio::task::JoinSet::new();
    let mut item_iter = items.into_iter().peekable();
    let mut first_error: Option<anyhow::Error> = None;

    loop {
        while first_error.is_none() && tasks.len() < concurrency && item_iter.peek().is_some() {
            let (idx, item) = item_iter.next().expect("peeked");
            tasks.spawn(worker(idx, item));
        }

        match tasks.join_next().await {
            Some(Ok(Ok((idx, value)))) => results[idx] = Some(value),
            Some(Ok(Err(e))) => first_error = Some(e),
            Some(Err(join_err)) => {
                first_error = Some(anyhow::anyhow!("task panicked: {join_err}"));
            }
            None => break,
        }

        if tasks.is_empty() && item_iter.peek().is_none() {
            break;
        }
    }

    (results, first_error)
}

/// Run the corpus through every arm using replay or live scoring, then judge
/// the held-out split when live.
///
/// If `resume` is provided, (arm, variant_id) pairs recorded as completed are
/// skipped so the run continues from the exhaustion point.
pub async fn run_corpus(
    corpus: &Corpus,
    config: &RunnerConfig,
    resume: Option<&ResumeState>,
) -> anyhow::Result<LiveRunResult> {
    let summarizer: Arc<dyn FlashSummarizer> =
        match build_summarizer(config.mode, SelectionArm::FlashSummarization) {
            Ok(s) => s,
            Err(e) => {
                return Ok(hard_error_result(
                    "leg_1",
                    Vec::new(),
                    Vec::new(),
                    HashSet::new(),
                    &e,
                ));
            }
        };
    let jev_client = if config.mode == RunnerMode::Live {
        match build_jev_client(config.endpoint) {
            Ok((client, configured_model)) => Some((Arc::new(client), configured_model)),
            Err(e) => {
                return Ok(hard_error_result(
                    "leg_1",
                    Vec::new(),
                    Vec::new(),
                    HashSet::new(),
                    &anyhow::anyhow!("{e}"),
                ));
            }
        }
    } else {
        None
    };

    let mut completed: HashSet<CompletedKey> = resume
        .map(|r| r.completed.iter().cloned().collect())
        .unwrap_or_default();

    // Build the deterministic work list, skipping pairs already recorded in a
    // resume state. Indices are contiguous after filtering so output slots are
    // dense.
    let work_items: Vec<_> = corpus
        .calibration
        .iter()
        .chain(corpus.held_out.iter())
        .flat_map(|edge| {
            SelectionArm::all().iter().map(move |arm| {
                let key = CompletedKey::new(arm.name(), &edge.variant_id);
                (edge.clone(), *arm, key)
            })
        })
        .filter(|(_, _, key)| !completed.contains(key))
        .enumerate()
        .map(|(idx, (edge, arm, _key))| (idx, (edge, arm)))
        .collect();
    let mut scored = Vec::new();

    let concurrency = run_concurrency();
    let worker_config = config.clone();
    let summarizer = Arc::clone(&summarizer);
    let jev_client = jev_client.clone();

    let (results, first_error) = run_bounded(
        work_items,
        concurrency,
        move |idx: usize, (edge, arm): (LabelledEdge, SelectionArm)| {
            let config = worker_config.clone();
            let summarizer = Arc::clone(&summarizer);
            let jev_client = jev_client.clone();
            async move {
                let state = assemble_state(&edge.edge, arm, &config.arm_config, &*summarizer)
                    .await
                    .map_err(|e| wrap_state_exhaustion(e, arm))?;
                let scored_edge = score_one(&edge, arm, &state, &config, jev_client.as_ref())
                    .await
                    .map_err(|e| {
                        if is_exhaustion(&e) {
                            anyhow::anyhow!("jev_scorer exhausted: {e}")
                        } else {
                            e
                        }
                    })?;
                let key = CompletedKey::new(arm.name(), &edge.variant_id);
                Ok((idx, (scored_edge, key)))
            }
        },
    )
    .await;

    for (scored_edge, key) in results.into_iter().flatten() {
        completed.insert(key);
        scored.push(scored_edge);
    }

    if let Some(e) = first_error {
        if is_exhaustion(&e) {
            return Ok(LiveRunResult {
                scored,
                judge_results: Vec::new(),
                resume: Some(ResumeState::exhausted(
                    exhausted_service(&e),
                    completed.into_iter().collect(),
                    &e.to_string(),
                )),
            });
        }
        return Ok(hard_error_result(
            "leg_1",
            scored,
            Vec::new(),
            completed,
            &e,
        ));
    }

    let (judge_results, judge_error) = if config.mode == RunnerMode::Live {
        run_judge(corpus, config, &scored, &mut completed).await
    } else {
        (Vec::new(), None)
    };

    if let Some(e) = judge_error {
        if is_exhaustion(&e) {
            return Ok(LiveRunResult {
                scored,
                judge_results,
                resume: Some(ResumeState::exhausted(
                    exhausted_service(&e),
                    completed.into_iter().collect(),
                    &e.to_string(),
                )),
            });
        }
        return Ok(hard_error_result(
            "leg_2",
            scored,
            judge_results,
            completed,
            &e,
        ));
    }

    Ok(LiveRunResult {
        scored,
        judge_results,
        resume: None,
    })
}

fn wrap_state_exhaustion(e: anyhow::Error, arm: SelectionArm) -> anyhow::Error {
    if is_exhaustion(&e) {
        let service = if matches!(arm, SelectionArm::FlashSummarization) {
            "summarizer"
        } else {
            "state_assembly"
        };
        anyhow::anyhow!("{service} exhausted: {e}")
    } else {
        e
    }
}

fn exhausted_service(err: &anyhow::Error) -> &'static str {
    let msg = err.to_string();
    if msg.starts_with("summarizer exhausted:") {
        "summarizer"
    } else if msg.starts_with("state_assembly exhausted:") {
        "state_assembly"
    } else if msg.starts_with("judge exhausted:") {
        "judge"
    } else {
        "jev_scorer"
    }
}

fn hard_error_result(
    leg: &str,
    scored: Vec<ScoredEdge>,
    judge_results: Vec<(String, SelectionArm, JudgeVerdict)>,
    completed: HashSet<CompletedKey>,
    err: &anyhow::Error,
) -> LiveRunResult {
    LiveRunResult {
        scored,
        judge_results,
        resume: Some(ResumeState::hard_error(
            format!("hard_error:{leg}"),
            completed.into_iter().collect(),
            &err.to_string(),
        )),
    }
}

fn build_summarizer(
    mode: RunnerMode,
    arm: SelectionArm,
) -> anyhow::Result<Arc<dyn FlashSummarizer>> {
    if mode == RunnerMode::Live && matches!(arm, SelectionArm::FlashSummarization) {
        let config = SummarizerConfig::from_env()?;
        Ok(Arc::new(LiveSummarizer::new(config)?))
    } else {
        Ok(Arc::new(FixtureSummarizer))
    }
}

async fn run_judge(
    corpus: &Corpus,
    config: &RunnerConfig,
    scored: &[ScoredEdge],
    completed: &mut HashSet<CompletedKey>,
) -> (
    Vec<(String, SelectionArm, JudgeVerdict)>,
    Option<anyhow::Error>,
) {
    let judge_config = match JudgeConfig::from_env() {
        Ok(c) => c,
        Err(e) => return (Vec::new(), Some(e)),
    };
    let client = match JudgeClient::new(judge_config) {
        Ok(c) => c,
        Err(e) => return (Vec::new(), Some(e)),
    };
    let held_out_ids: HashSet<String> = corpus
        .held_out
        .iter()
        .map(|e| e.variant_id.clone())
        .collect();

    let mut edges_to_judge = Vec::new();
    for edge in scored {
        if held_out_ids.contains(&edge.variant_id) {
            // Re-find the original LabelledEdge for state assembly.
            if let Some(labelled) = corpus
                .held_out
                .iter()
                .find(|e| e.variant_id == edge.variant_id)
            {
                let key = CompletedKey::judge(edge.arm.name(), &edge.variant_id);
                if completed.contains(&key) {
                    continue;
                }
                edges_to_judge.push((labelled.clone(), edge.arm));
            }
        }
    }

    judge_edges_bounded(&client, edges_to_judge, &config.arm_config, Some(completed)).await
}

fn default_response() -> ScriptedResponse {
    let mut answers = BTreeMap::new();
    answers.insert(
        "evidence_presence".to_owned(),
        WireAnswer::Noul { noul: 0.9 },
    );
    answers.insert(
        "task_fidelity".to_owned(),
        WireAnswer::Score {
            score: 3.0,
            legend: BTreeMap::from([
                ("0".to_owned(), "none".to_owned()),
                ("1".to_owned(), "poor".to_owned()),
                ("2".to_owned(), "fair".to_owned()),
                ("3".to_owned(), "good".to_owned()),
                ("4".to_owned(), "excellent".to_owned()),
            ]),
            probabilities: BTreeMap::from([
                ("0".to_owned(), 0.1),
                ("1".to_owned(), 0.2),
                ("2".to_owned(), 0.4),
                ("3".to_owned(), 0.2),
                ("4".to_owned(), 0.1),
            ]),
            confidence: 0.7,
        },
    );
    answers.insert(
        "goal_alignment".to_owned(),
        WireAnswer::Score {
            score: 1.0,
            legend: BTreeMap::from([
                ("0".to_owned(), "misaligned".to_owned()),
                ("1".to_owned(), "neutral".to_owned()),
                ("2".to_owned(), "aligned".to_owned()),
            ]),
            probabilities: BTreeMap::from([
                ("0".to_owned(), 0.2),
                ("1".to_owned(), 0.6),
                ("2".to_owned(), 0.2),
            ]),
            confidence: 0.7,
        },
    );
    answers.insert(
        "evidence_grounding".to_owned(),
        WireAnswer::Score {
            score: 2.0,
            legend: BTreeMap::from([
                ("0".to_owned(), "unsupported".to_owned()),
                ("1".to_owned(), "partially".to_owned()),
                ("2".to_owned(), "mostly".to_owned()),
                ("3".to_owned(), "supported".to_owned()),
            ]),
            probabilities: BTreeMap::from([
                ("0".to_owned(), 0.1),
                ("1".to_owned(), 0.2),
                ("2".to_owned(), 0.5),
                ("3".to_owned(), 0.2),
            ]),
            confidence: 0.7,
        },
    );
    answers.insert(
        "handoff_integrity".to_owned(),
        WireAnswer::Noul { noul: 0.8 },
    );
    answers.insert(
        "scope_discipline".to_owned(),
        WireAnswer::Noul { noul: 0.8 },
    );
    ScriptedResponse {
        model: "jev-1.13.0".to_owned(),
        answers,
        usage: WireUsage {
            input_tokens: 2048,
            output_tokens: 128,
        },
    }
}

/// Class-aware synthetic replay response.
///
/// These answers model what a plausible scorer would return for each failure
/// class. They are test data for the scoring math, not model predictions.
/// Fixtures are deterministic by board ruling (2026-10-08); no noise term is
/// added, so the offline phase exercises the scoring math reproducibly.
#[must_use]
pub fn class_aware_response(class: FailureClass, arm: SelectionArm) -> ScriptedResponse {
    // Arm modulation: head-truncation makes the scorer slightly less certain
    // about grounding because evidence was removed from the prompt.
    let grounding_penalty: f64 = if matches!(arm, SelectionArm::HeadTruncation) {
        1.0
    } else {
        0.0
    };

    let (presence, fidelity, alignment, mut grounding, handoff, scope) = match class {
        FailureClass::Clean | FailureClass::LegitimateScopeOverlap => {
            (0.95, 4.0, 2.0, 3.0, 0.9, 0.9)
        }
        FailureClass::Drifted => (0.8, 1.0, 0.0, 2.0, 0.7, 0.8),
        FailureClass::FabricatedSpecifics => (0.85, 3.0, 2.0, 1.0, 0.8, 0.8),
        FailureClass::ScopeViolation => (0.8, 2.0, 1.0, 2.0, 0.7, 0.1),
        FailureClass::LossyHandoff => (0.8, 3.0, 2.0, 2.0, 0.1, 0.8),
        FailureClass::DistractorAligned => (0.7, 1.0, 0.0, 1.0, 0.7, 0.8),
        FailureClass::MissingEvidence => (0.1, 2.0, 1.0, 1.0, 0.5, 0.8),
        FailureClass::ContradictorySources => (0.8, 3.0, 1.0, 0.0, 0.7, 0.8),
        FailureClass::Truncation => (0.9, 4.0, 2.0, 3.0, 0.9, 0.9),
        FailureClass::AdversarialInjection => (0.99, 4.0, 2.0, 1.0, 0.99, 0.99),
    };
    grounding = (grounding - grounding_penalty).clamp(0.0, 3.0);

    let mut answers = BTreeMap::new();
    answers.insert(
        "evidence_presence".to_owned(),
        WireAnswer::Noul { noul: presence },
    );
    answers.insert(
        "task_fidelity".to_owned(),
        score_answer(fidelity, &["none", "poor", "fair", "good", "excellent"]),
    );
    answers.insert(
        "goal_alignment".to_owned(),
        score_answer(alignment, &["misaligned", "neutral", "aligned"]),
    );
    answers.insert(
        "evidence_grounding".to_owned(),
        score_answer(
            grounding,
            &["unsupported", "partially", "mostly", "supported"],
        ),
    );
    answers.insert(
        "handoff_integrity".to_owned(),
        WireAnswer::Noul { noul: handoff },
    );
    answers.insert(
        "scope_discipline".to_owned(),
        WireAnswer::Noul { noul: scope },
    );

    ScriptedResponse {
        model: "jev-1.13.0-replay".to_owned(),
        answers,
        usage: WireUsage {
            input_tokens: 2048,
            output_tokens: 128,
        },
    }
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "labels.len() is tiny (<=5) and score is a valid level index"
)]
fn score_answer(score: f64, labels: &[&str]) -> WireAnswer {
    let legend: BTreeMap<String, String> = labels
        .iter()
        .enumerate()
        .map(|(i, label)| (i.to_string(), (*label).to_owned()))
        .collect();
    let chosen = score.round() as usize;
    let n = labels.len().max(1);
    let residual = 1.0 - 0.7;
    let probabilities: BTreeMap<String, f64> = (0..labels.len())
        .map(|i| {
            let p = if i == chosen {
                0.7
            } else {
                residual / (n - 1) as f64
            };
            (i.to_string(), p)
        })
        .collect();
    WireAnswer::Score {
        score,
        legend,
        probabilities,
        confidence: 0.8,
    }
}

async fn score_one(
    labelled: &LabelledEdge,
    arm: SelectionArm,
    state: &EdgeState,
    config: &RunnerConfig,
    live_client: Option<&(Arc<JevClient>, String)>,
) -> anyhow::Result<ScoredEdge> {
    let started = Instant::now();

    let extraction_started = Instant::now();
    let claims = extract_claims(&state.worker_output.result);
    let full_capture = labelled.edge.full_capture();
    let (_, grounding_stats) = resolve_claims(&claims, &full_capture, &state.context_provided);
    let extraction_ms = extraction_started.elapsed().as_millis() as u64;

    let retrieval_started = Instant::now();
    let retrieval_ms = retrieval_started.elapsed().as_millis() as u64;

    let summarization_ms = if matches!(arm, SelectionArm::FlashSummarization) {
        5
    } else {
        0
    };

    let request = EdgeRubric::request().state(state)?;
    let decision = request.build()?;

    let (answers, response_model, usage) = if let Some((client, configured_model)) = live_client {
        let raw = client.evaluate_raw(&decision).await?;
        let model = resolve_model_id(&raw.model, configured_model);
        let answers = EdgeRubricAnswers::from_answers(&raw)?;
        (
            answers,
            model,
            TokenUsage {
                input_tokens: raw.usage.input_tokens,
                output_tokens: raw.usage.output_tokens,
            },
        )
    } else {
        let response = class_aware_response(labelled.class, arm);
        let transport = Arc::new(ScriptedTransport::new(vec![Ok(WireResponse {
            model: response.model.clone(),
            answers: response.answers.clone(),
            usage: response.usage,
        })]));
        let client =
            JevClient::with_transport(transport, config.model_id.clone(), RetryConfig::default());
        let answers: EdgeRubricAnswers = client.evaluate(&decision).await?;
        (
            answers,
            response.model,
            TokenUsage {
                input_tokens: response.usage.input_tokens,
                output_tokens: response.usage.output_tokens,
            },
        )
    };
    let jev_ms = started.elapsed().as_millis() as u64;

    let fan_out_ms = 0u64;
    let state_hash = hash_state(state);
    let truncation_flags = TruncationFlags {
        goal: state.goal.len() >= config.arm_config.goal_cap,
        plan_step: state.plan_step.task.len() >= config.arm_config.plan_step_cap,
        plan_context: false,
        worker_output: state.worker_output.truncated,
        context_provided: state.context_provided.len() >= config.arm_config.context_provided_cap,
        evidence: state.claim_evidence.iter().any(|c| c.truncated),
    };

    let (composite, flags) = score_composite(&answers, &config.weights, &truncation_flags);
    let verdict = resolve_verdict(&answers, composite, &flags, &config.weights);

    let total_ms = started.elapsed().as_millis() as u64;
    let artifact = VerdictArtifact {
        rubric_version: "v2".to_owned(),
        weights_version: "v2-provisional".to_owned(),
        weights: config.weights,
        resolved_model_id: response_model,
        state_hash,
        answers: crate::verdict::AnswerSummary::from(&answers),
        grounding_stats,
        composite,
        verdict,
        truncation_flags,
        latency_ms: crate::verdict::StageLatencies {
            extraction_ms,
            retrieval_ms,
            summarization_ms,
            jev_ms,
            fan_out_ms,
            write_ms: 0,
            total_ms,
        },
        tokens: TokenUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
        },
        timestamp: format_system_time(SystemTime::now()),
    };

    Ok(ScoredEdge {
        variant_id: labelled.variant_id.clone(),
        class: labelled.class,
        arm,
        artifact,
    })
}

fn format_system_time(time: SystemTime) -> String {
    let secs = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    format!("{secs}")
}

/// Write every scored edge artifact to a directory.
pub fn write_artifacts(scored: &[ScoredEdge], dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    for edge in scored {
        let filename = format!("{}-{:?}.json", edge.variant_id, edge.arm)
            .to_lowercase()
            .replace(' ', "_");
        let path = dir.join(filename);
        let json = serde_json::to_string_pretty(edge)?;
        std::fs::write(path, json)?;
    }
    Ok(())
}

/// Write judge verdicts to a single file in the artifact directory.
pub fn write_judge_results(
    results: &[(String, SelectionArm, JudgeVerdict)],
    dir: &Path,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("judge_results.json");
    let json = serde_json::to_string_pretty(results)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Convenience wrapper for backward-compatible replay-only tests.
pub async fn replay_corpus(
    corpus: &Corpus,
    config: &RunnerConfig,
) -> anyhow::Result<Vec<ScoredEdge>> {
    let result = run_corpus(corpus, config, None).await?;
    Ok(result.scored)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use tokio::sync::Semaphore;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    use super::{RunnerConfig, RunnerMode, run_bounded, run_concurrency, run_corpus};
    use crate::arms::SelectionArm;
    use crate::client::JevEndpoint;
    use crate::corpus::{CapturedEdge, Corpus, FailureClass, LabelledEdge};
    use crate::exhaustion::is_exhaustion;
    use crate::state::{PlanContext, PlanStep, WorkerOutput};

    #[tokio::test]
    async fn run_bounded_preserves_input_order_despite_completion_order() {
        let items: Vec<_> = (0..8).map(|i| (i, i)).collect();

        let (results, error) = run_bounded(items, 4, |idx: usize, value: usize| async move {
            // Later indices finish first; the helper must still return
            // values in original index order.
            tokio::time::sleep(Duration::from_millis((8 - value) as u64 * 5)).await;
            Ok((idx, value * 10))
        })
        .await;

        assert!(error.is_none());
        let collected: Vec<_> = results.into_iter().flatten().collect();
        assert_eq!(collected, vec![0, 10, 20, 30, 40, 50, 60, 70]);
    }

    #[tokio::test]
    async fn run_bounded_respects_concurrency_limit() {
        let items: Vec<_> = (0..12).map(|i| (i, i)).collect();
        let limit = Arc::new(Semaphore::new(4));
        let max_in_flight = Arc::new(AtomicUsize::new(0));
        let current = Arc::new(AtomicUsize::new(0));

        let limit_clone = Arc::clone(&limit);
        let max_clone = Arc::clone(&max_in_flight);
        let current_clone = Arc::clone(&current);

        let (_results, error) = run_bounded(items, 4, move |idx: usize, value: usize| {
            let limit = Arc::clone(&limit_clone);
            let max = Arc::clone(&max_clone);
            let current = Arc::clone(&current_clone);
            async move {
                let _permit = limit.acquire().await.expect("semaphore closed");
                let n = current.fetch_add(1, Ordering::SeqCst) + 1;
                max.fetch_max(n, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
                current.fetch_sub(1, Ordering::SeqCst);
                Ok((idx, value))
            }
        })
        .await;

        assert!(error.is_none());
        assert_eq!(max_in_flight.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn run_bounded_drains_in_flight_on_exhaustion() {
        // With concurrency 4, items 0-3 are in flight. Item 3 exhausts quickly
        // while 0-2 sleep; scheduling stops, but 0-2 are allowed to finish.
        let items: Vec<_> = (0..8).map(|i| (i, i)).collect();

        let (results, error) = run_bounded(items, 4, |idx: usize, value: usize| async move {
            if value == 3 {
                return Err(anyhow::anyhow!("exhausted: 429 rate limited"));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok((idx, value))
        })
        .await;

        assert!(error.is_some());
        assert!(error.unwrap().to_string().contains("429"));

        // Items 0-2 should have completed; item 3 failed; items 4-7 were never
        // scheduled.
        for (i, slot) in results.iter().enumerate().take(3) {
            assert_eq!(*slot, Some(i), "item {i} should have completed");
        }
        assert!(results[3].is_none(), "exhausted item should not complete");
        for (i, slot) in results.iter().enumerate().skip(4) {
            assert!(slot.is_none(), "item {i} should not have been scheduled");
        }
    }

    #[tokio::test]
    async fn run_concurrency_env_override() {
        static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let _guard = ENV_LOCK.lock().await;

        unsafe {
            std::env::set_var("RUN_CONCURRENCY", "2");
        }
        assert_eq!(run_concurrency(), 2);
        unsafe {
            std::env::remove_var("RUN_CONCURRENCY");
        }
        assert_eq!(run_concurrency(), 4);

        unsafe {
            std::env::set_var("RUN_CONCURRENCY", "0");
        }
        assert_eq!(run_concurrency(), 1);
        unsafe {
            std::env::remove_var("RUN_CONCURRENCY");
        }
    }

    fn dummy_edge(variant_id: &str, result: &str) -> LabelledEdge {
        LabelledEdge {
            variant_id: variant_id.to_owned(),
            class: FailureClass::Clean,
            edge: CapturedEdge {
                scenario: "test".to_owned(),
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
                    result: result.to_owned(),
                    truncated: false,
                },
                tool_calls: Vec::new(),
            },
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

    struct JevCountResponder {
        attempt: AtomicUsize,
        successes: usize,
        ok_body: serde_json::Value,
    }

    impl JevCountResponder {
        fn ok_for_first(successes: usize, ok_body: serde_json::Value) -> Self {
            Self {
                attempt: AtomicUsize::new(0),
                successes,
                ok_body,
            }
        }
    }

    impl Respond for JevCountResponder {
        fn respond(&self, _request: &Request) -> ResponseTemplate {
            let n = self.attempt.fetch_add(1, Ordering::SeqCst);
            if n < self.successes {
                ResponseTemplate::new(200).set_body_json(self.ok_body.clone())
            } else {
                ResponseTemplate::new(500).set_body_string("boom")
            }
        }
    }

    #[tokio::test]
    async fn hard_error_in_scoring_preserves_completed_pairs() {
        static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let _guard = ENV_LOCK.lock().await;

        let jev_server = MockServer::start().await;
        let sum_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"role": "assistant", "content": "summary"}}]
            })))
            .mount(&sum_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .respond_with(JevCountResponder::ok_for_first(3, jev_ok_response()))
            .expect(4)
            .mount(&jev_server)
            .await;

        unsafe {
            std::env::set_var("RUN_CONCURRENCY", "1");
            std::env::set_var("TYPESAFE_BASE_URL", jev_server.uri());
            std::env::set_var("TYPESAFE_API_KEY", "test-key");
            std::env::set_var("TYPESAFE_MODEL", "jev-1.13.0");
            std::env::set_var("SUMMARIZER_BASE_URL", sum_server.uri());
            std::env::set_var("SUMMARIZER_API_KEY", "test-key");
            std::env::set_var("SUMMARIZER_MODEL", "test-model");
        }

        let corpus = Corpus {
            calibration: vec![
                dummy_edge("v0", "result zero"),
                dummy_edge("v1", "result one"),
            ],
            held_out: Vec::new(),
        };
        let config = RunnerConfig {
            mode: RunnerMode::Live,
            endpoint: JevEndpoint::Cloud,
            ..Default::default()
        };

        let result = run_corpus(&corpus, &config, None)
            .await
            .unwrap_or_else(|e| panic!("{e}"));

        assert_eq!(
            result.scored.len(),
            3,
            "first edge's three arms should be preserved"
        );
        let resume = result.resume.expect("expected resume state");
        assert!(
            resume.error.starts_with("hard_error:leg_1"),
            "unexpected error: {}",
            resume.error
        );
        assert!(!is_exhaustion(&anyhow::anyhow!("{}", resume.error)));
        assert_eq!(resume.completed.len(), 3);
        for arm in SelectionArm::all() {
            assert!(resume.is_completed(arm.name(), "v0"));
        }

        unsafe {
            std::env::remove_var("RUN_CONCURRENCY");
            std::env::remove_var("TYPESAFE_BASE_URL");
            std::env::remove_var("TYPESAFE_API_KEY");
            std::env::remove_var("TYPESAFE_MODEL");
            std::env::remove_var("SUMMARIZER_BASE_URL");
            std::env::remove_var("SUMMARIZER_API_KEY");
            std::env::remove_var("SUMMARIZER_MODEL");
        }
    }
}
