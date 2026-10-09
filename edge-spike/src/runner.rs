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
use crate::judge::{JudgeClient, JudgeConfig, JudgeVerdict, judge_held_out};
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
        build_summarizer(config.mode, SelectionArm::FlashSummarization)?;
    let jev_client = if config.mode == RunnerMode::Live {
        let (client, configured_model) = build_jev_client(config.endpoint)?;
        Some((client, configured_model))
    } else {
        None
    };

    let mut completed: HashSet<CompletedKey> = resume
        .map(|r| r.completed.iter().cloned().collect())
        .unwrap_or_default();
    let mut scored = Vec::new();

    for edge in corpus.calibration.iter().chain(corpus.held_out.iter()) {
        for arm in SelectionArm::all() {
            let arm_name = arm.name();
            let key = CompletedKey::new(arm_name, &edge.variant_id);
            if completed.contains(&key) {
                continue;
            }

            let state =
                match assemble_state(&edge.edge, *arm, &config.arm_config, &*summarizer).await {
                    Ok(s) => s,
                    Err(e) if is_exhaustion(&e) => {
                        return Ok(LiveRunResult {
                            scored,
                            judge_results: Vec::new(),
                            resume: Some(ResumeState::exhausted(
                                if matches!(arm, SelectionArm::FlashSummarization) {
                                    "summarizer"
                                } else {
                                    "state_assembly"
                                },
                                completed.into_iter().collect(),
                                &e.to_string(),
                            )),
                        });
                    }
                    Err(e) => return Err(e),
                };
            let result = score_one(edge, *arm, &state, config, jev_client.as_ref()).await;
            match result {
                Ok(s) => {
                    completed.insert(key);
                    scored.push(s);
                }
                Err(e) if is_exhaustion(&e) => {
                    return Ok(LiveRunResult {
                        scored,
                        judge_results: Vec::new(),
                        resume: Some(ResumeState::exhausted(
                            "jev_scorer",
                            completed.into_iter().collect(),
                            &e.to_string(),
                        )),
                    });
                }
                Err(e) => return Err(e),
            }
        }
    }

    let judge_results = if config.mode == RunnerMode::Live {
        run_judge(corpus, config, &scored).await?
    } else {
        Vec::new()
    };

    Ok(LiveRunResult {
        scored,
        judge_results,
        resume: None,
    })
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
) -> anyhow::Result<Vec<(String, SelectionArm, JudgeVerdict)>> {
    let judge_config = JudgeConfig::from_env()?;
    let client = JudgeClient::new(judge_config)?;
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
                edges_to_judge.push((labelled, edge.arm));
            }
        }
    }

    judge_held_out(&client, &edges_to_judge, &config.arm_config, &held_out_ids).await
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
    live_client: Option<&(JevClient, String)>,
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
