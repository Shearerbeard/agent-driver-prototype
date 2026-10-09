//! Replay/live runner: corpus x arms scored through replay fixtures or a real
//! Jev client, plus optional Leg-2 judge over the held-out split.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs::OpenOptions;
use std::io::Write;
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
use crate::corpus::{CapturedEdge, Corpus, FailureClass, LabelledEdge};
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

/// Which variants the runner scores.
#[derive(Debug, Clone, Default)]
pub enum RunSelector {
    /// Score every variant in the corpus (default).
    #[default]
    All,
    /// Score exactly one variant across the selected arms.
    Edge(String),
    /// Score one deterministic clean calibration edge.
    Smoke,
    /// Score a deterministic stratified slice of the calibration clean edges.
    Slice(usize),
}

impl RunSelector {
    /// Validate selector arguments. Fails loud on invalid inputs.
    pub fn validate(&self) -> anyhow::Result<()> {
        match self {
            RunSelector::Slice(n) if *n == 0 => Err(anyhow::anyhow!("--slice requires N > 0")),
            _ => Ok(()),
        }
    }
}

/// Runner configuration.
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    pub weights: Weights,
    pub arm_config: ArmConfig,
    pub model_id: String,
    pub mode: RunnerMode,
    pub endpoint: JevEndpoint,
    /// Arms to run. Default is all three.
    pub arms: Vec<SelectionArm>,
    /// Variant subset selector.
    pub selector: RunSelector,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            weights: Weights::default(),
            arm_config: ArmConfig::default(),
            model_id: "jev-1.13.0".to_owned(),
            mode: RunnerMode::Replay,
            endpoint: JevEndpoint::Cloud,
            arms: SelectionArm::all().to_vec(),
            selector: RunSelector::All,
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

/// JSONL file name for durable Leg-1 scoring records.
pub const RESULTS_JSONL: &str = "results.jsonl";

/// JSONL file name for durable Leg-2 judge records.
pub const JUDGE_RESULTS_JSONL: &str = "judge_results.jsonl";

/// One line of `judge_results.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeResultRecord {
    pub variant_id: String,
    pub arm: SelectionArm,
    pub verdict: JudgeVerdict,
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
pub fn plan_run(corpus: &Corpus, config: &RunnerConfig) -> anyhow::Result<RunPlan> {
    let mut plan = RunPlan::default();
    let input_per_call = 2048u64;
    let output_per_call = 128u64;

    let selected = select_variants(corpus, &config.selector)?;

    for _edge in &selected {
        for arm in &config.arms {
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
        let held_out_ids: HashSet<&str> = corpus
            .held_out
            .iter()
            .map(|edge| edge.variant_id.as_str())
            .collect();
        let judge_edges = selected
            .iter()
            .filter(|edge| held_out_ids.contains(edge.variant_id.as_str()))
            .count();
        plan.judge_calls = judge_edges.saturating_mul(config.arms.len());
    }

    Ok(plan)
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

/// Resolve the selector to the list of variants that will be scored.
///
/// Deterministic ordering is required for `--smoke` and `--slice` to be
/// reproducible across runs and filesystems.
fn select_variants<'a>(
    corpus: &'a Corpus,
    selector: &RunSelector,
) -> anyhow::Result<Vec<&'a LabelledEdge>> {
    match selector {
        RunSelector::All => Ok(corpus
            .calibration
            .iter()
            .chain(corpus.held_out.iter())
            .collect()),
        RunSelector::Edge(id) => select_edge(corpus, id),
        RunSelector::Smoke => select_smoke(corpus),
        RunSelector::Slice(n) => select_slice(corpus, *n),
    }
}

fn select_edge<'a>(corpus: &'a Corpus, id: &str) -> anyhow::Result<Vec<&'a LabelledEdge>> {
    corpus
        .calibration
        .iter()
        .chain(corpus.held_out.iter())
        .find(|e| e.variant_id == id)
        .map(|e| vec![e])
        .ok_or_else(|| anyhow::anyhow!("unknown variant id: {id}"))
}

fn calibration_sorted(corpus: &Corpus) -> Vec<&LabelledEdge> {
    let mut edges: Vec<&LabelledEdge> = corpus.calibration.iter().collect();
    edges.sort_by_key(|e| e.variant_id.clone());
    edges
}

fn select_smoke(corpus: &Corpus) -> anyhow::Result<Vec<&LabelledEdge>> {
    let edge = calibration_sorted(corpus)
        .into_iter()
        .filter(|e| e.class == FailureClass::Clean)
        .find(|e| {
            e.edge.tool_calls.len() >= 5
                && e.edge.tool_calls.iter().any(|c| !c.result.contains(" [truncated]"))
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no clean calibration edge satisfies smoke criteria (>=5 tool calls, >=1 non-truncated result)"
            )
        })?;
    println!(
        "smoke: variant_id={} scenario={} worker={} tool_calls={} basis=first calibration Clean variant with >=5 tool calls and >=1 non-truncated result",
        edge.variant_id,
        edge.edge.scenario,
        edge.edge.worker_id,
        edge.edge.tool_calls.len()
    );
    Ok(vec![edge])
}

fn select_slice(corpus: &Corpus, n: usize) -> anyhow::Result<Vec<&LabelledEdge>> {
    let mut by_scenario: BTreeMap<String, Vec<&LabelledEdge>> = BTreeMap::new();
    for edge in calibration_sorted(corpus) {
        if edge.class == FailureClass::Clean {
            by_scenario
                .entry(edge.edge.scenario.clone())
                .or_default()
                .push(edge);
        }
    }
    for edges in by_scenario.values_mut() {
        edges.sort_by_key(|e| e.variant_id.clone());
    }

    let scenarios: Vec<String> = by_scenario.keys().cloned().collect();
    let mut selected = Vec::new();
    let mut pass = 0usize;
    'outer: loop {
        for scenario in &scenarios {
            if selected.len() >= n {
                break 'outer;
            }
            if let Some(edges) = by_scenario.get(scenario)
                && pass < edges.len()
            {
                selected.push(edges[pass]);
            }
        }
        pass += 1;
        if selected.len() >= n {
            break;
        }
        let any_remaining = scenarios
            .iter()
            .any(|s| by_scenario.get(s).map(|e| e.len()).unwrap_or(0) > pass);
        if !any_remaining {
            break;
        }
    }

    if selected.len() < n {
        anyhow::bail!(
            "--slice {n} requested but only {} clean calibration variants available",
            selected.len()
        );
    }

    let ids: Vec<&str> = selected.iter().map(|e| e.variant_id.as_str()).collect();
    println!(
        "slice: selected {} variants: {}",
        selected.len(),
        ids.join(", ")
    );
    Ok(selected)
}

/// Load the set of completed scoring keys from `results.jsonl`, truncating a
/// torn final line if one is present.
pub fn load_completed_from_results_jsonl(path: &Path) -> anyhow::Result<HashSet<CompletedKey>> {
    load_completed_from_jsonl::<ScoredEdge, _>(path, |edge| {
        CompletedKey::new(edge.arm.name(), &edge.variant_id)
    })
}

/// Load the set of completed judge keys from `judge_results.jsonl`, truncating
/// a torn final line if one is present.
pub fn load_completed_from_judge_results_jsonl(
    path: &Path,
) -> anyhow::Result<HashSet<CompletedKey>> {
    load_completed_from_jsonl::<JudgeResultRecord, _>(path, |record| {
        CompletedKey::judge(record.arm.name(), &record.variant_id)
    })
}

fn load_completed_from_jsonl<T, F>(path: &Path, f: F) -> anyhow::Result<HashSet<CompletedKey>>
where
    T: serde::de::DeserializeOwned,
    F: Fn(T) -> CompletedKey,
{
    if !path.exists() {
        return Ok(HashSet::new());
    }
    let bytes = std::fs::read(path)?;
    let mut completed = HashSet::new();
    let mut start = 0usize;
    let mut last_valid_end = 0usize;
    let n = bytes.len();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            let line = &bytes[start..i];
            start = i + 1;
            if line.is_empty() {
                last_valid_end = start;
                continue;
            }
            match serde_json::from_slice::<T>(line) {
                Ok(item) => {
                    completed.insert(f(item));
                    last_valid_end = start;
                }
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "{} invalid line: {e}",
                        path.file_name().and_then(|s| s.to_str()).unwrap_or("jsonl")
                    ));
                }
            }
        }
    }
    if start < n {
        // Trailing content without a trailing newline is treated as a torn
        // final line and discarded so the run can re-score it.
        let file = OpenOptions::new().write(true).open(path)?;
        file.set_len(last_valid_end as u64)?;
    }
    Ok(completed)
}

/// Append one scored edge to `results.jsonl` and flush before returning.
pub fn append_scored_edge(path: &Path, edge: &ScoredEdge) -> anyhow::Result<()> {
    let mut file = OpenOptions::new().append(true).create(true).open(path)?;
    let mut line = serde_json::to_string(edge)?;
    line.push('\n');
    file.write_all(line.as_bytes())?;
    file.flush()?;
    Ok(())
}

/// Append one judge result to `judge_results.jsonl` and flush before returning.
pub fn append_judge_result(
    path: &Path,
    variant_id: &str,
    arm: SelectionArm,
    verdict: &JudgeVerdict,
) -> anyhow::Result<()> {
    let mut file = OpenOptions::new().append(true).create(true).open(path)?;
    let record = JudgeResultRecord {
        variant_id: variant_id.to_owned(),
        arm,
        verdict: verdict.clone(),
    };
    let mut line = serde_json::to_string(&record)?;
    line.push('\n');
    file.write_all(line.as_bytes())?;
    file.flush()?;
    Ok(())
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
/// If `output_dir` is provided, each completed scoring and judge call is
/// appended to `results.jsonl` / `judge_results.jsonl` and the completed-key
/// set is reconstructed from those logs on start, so a halt loses at most the
/// items still in flight.
pub async fn run_corpus(
    corpus: &Corpus,
    config: &RunnerConfig,
    output_dir: Option<&Path>,
) -> anyhow::Result<LiveRunResult> {
    // Only Arm 3 needs a summarizer. When it is not selected, skip construction
    // entirely so an unused SUMMARIZER_* env cannot fail the whole run.
    let summarizer: Option<Arc<dyn FlashSummarizer>> =
        match build_summarizer(config.mode, &config.arms) {
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

    let mut completed: HashSet<CompletedKey> = if let Some(dir) = output_dir {
        load_completed_from_results_jsonl(&dir.join(RESULTS_JSONL))?
    } else {
        HashSet::new()
    };

    if config.arms.is_empty() {
        anyhow::bail!("no arms selected");
    }

    let selected = select_variants(corpus, &config.selector)?;

    // Build the deterministic work list, skipping pairs already recorded in a
    // resume state. Indices are contiguous after filtering so output slots are
    // dense.
    let work_items: Vec<_> = selected
        .iter()
        .copied()
        .flat_map(|edge| {
            config.arms.iter().map(move |arm| {
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
    let summarizer = summarizer.clone();
    let jev_client = jev_client.clone();

    let (results, first_error) = run_bounded(
        work_items,
        concurrency,
        move |idx: usize, (edge, arm): (LabelledEdge, SelectionArm)| {
            let config = worker_config.clone();
            let summarizer = summarizer.clone();
            let jev_client = jev_client.clone();
            async move {
                let state =
                    assemble_arm_state(&edge.edge, arm, &config.arm_config, summarizer.as_deref())
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

    let results_path = output_dir.map(|dir| dir.join(RESULTS_JSONL));
    for (scored_edge, key) in results.into_iter().flatten() {
        if let Some(path) = results_path.as_ref() {
            append_scored_edge(path, &scored_edge)?;
        }
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
        run_judge(corpus, &selected, config, &mut completed, output_dir).await
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

/// Construct the summarizer only when Arm 3 is selected.
///
/// Returns `None` when `arms` omits [`SelectionArm::FlashSummarization`], so a
/// live run without Arm 3 never reads the `SUMMARIZER_*` environment. When
/// Arm 3 is selected, live mode requires that environment and fails loud if it
/// is missing.
fn build_summarizer(
    mode: RunnerMode,
    arms: &[SelectionArm],
) -> anyhow::Result<Option<Arc<dyn FlashSummarizer>>> {
    if !arms.contains(&SelectionArm::FlashSummarization) {
        return Ok(None);
    }
    if mode == RunnerMode::Live {
        let config = SummarizerConfig::from_env()?;
        Ok(Some(Arc::new(LiveSummarizer::new(config)?)))
    } else {
        Ok(Some(Arc::new(FixtureSummarizer)))
    }
}

/// Assemble the arm state, requiring a summarizer only for Arm 3.
///
/// A missing summarizer at Arm 3 is a routing bug (Arm 3 was assembled without
/// being selected); fail loud there rather than silently substituting a fixture.
async fn assemble_arm_state(
    edge: &CapturedEdge,
    arm: SelectionArm,
    config: &ArmConfig,
    summarizer: Option<&dyn FlashSummarizer>,
) -> anyhow::Result<EdgeState> {
    if matches!(arm, SelectionArm::FlashSummarization) {
        let summarizer = summarizer.ok_or_else(unavailable_summarizer_error)?;
        assemble_state(edge, arm, config, summarizer).await
    } else {
        // Arms 1 and 2 ignore the summarizer argument entirely.
        assemble_state(edge, arm, config, &FixtureSummarizer).await
    }
}

fn unavailable_summarizer_error() -> anyhow::Error {
    SummarizerConfig::from_env().err().unwrap_or_else(|| {
        anyhow::anyhow!("Arm 3 (flash summarization) assembled without a summarizer")
    })
}

/// Build the judge work list from the selected variants intersected with the
/// held-out split, independent of what the current process has scored.
///
/// The list derives from the corpus and the selector, never from this run's
/// scored results. Already-completed judge keys are skipped.
fn build_judge_work_list(
    corpus: &Corpus,
    selected: &[&LabelledEdge],
    completed: &HashSet<CompletedKey>,
    arms: &[SelectionArm],
) -> Vec<(LabelledEdge, SelectionArm)> {
    let held_out_ids: HashSet<&str> = corpus
        .held_out
        .iter()
        .map(|edge| edge.variant_id.as_str())
        .collect();
    selected
        .iter()
        .copied()
        .filter(|edge| held_out_ids.contains(edge.variant_id.as_str()))
        .flat_map(|edge| {
            arms.iter().filter_map(move |&arm| {
                let key = CompletedKey::judge(arm.name(), &edge.variant_id);
                if completed.contains(&key) {
                    None
                } else {
                    Some((edge.clone(), arm))
                }
            })
        })
        .collect()
}

async fn run_judge(
    corpus: &Corpus,
    selected: &[&LabelledEdge],
    config: &RunnerConfig,
    completed: &mut HashSet<CompletedKey>,
    output_dir: Option<&Path>,
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

    if let Some(dir) = output_dir {
        match load_completed_from_judge_results_jsonl(&dir.join(JUDGE_RESULTS_JSONL)) {
            Ok(judge_completed) => completed.extend(judge_completed),
            Err(e) => return (Vec::new(), Some(e)),
        }
    }

    let edges_to_judge = build_judge_work_list(corpus, selected, completed, &config.arms);

    judge_edges_bounded(
        &client,
        edges_to_judge,
        &config.arm_config,
        Some(completed),
        output_dir,
    )
    .await
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
        // Honest blocker: the worker reported it could not fulfil the task.
        FailureClass::BaselineBlocked => (0.1, 1.0, 1.0, 1.0, 0.9, 0.9),
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
    let state_chars = state.char_len();
    let truncation_flags = TruncationFlags {
        goal: state.goal.len() >= config.arm_config.goal_cap,
        plan_step: state.plan_step.task.len() >= config.arm_config.plan_step_cap,
        plan_context: labelled.edge.plan_context.consumers.len()
            != state.plan_context.consumers.len()
            || labelled.edge.plan_context.siblings.len() != state.plan_context.siblings.len(),
        worker_output: state.worker_output.truncated,
        context_provided: state.context_provided.len() >= config.arm_config.context_provided_cap,
        evidence: state.claim_evidence.iter().any(|c| c.truncated),
        source: labelled.edge.worker_output.truncated
            || labelled
                .edge
                .tool_calls
                .iter()
                .any(|c| c.result.contains(" [truncated]")),
    };

    let (composite, flags) = score_composite(&answers, &config.weights, &truncation_flags);
    let verdict = resolve_verdict(&answers, composite, &flags, &config.weights);

    let total_ms = started.elapsed().as_millis() as u64;
    let artifact = VerdictArtifact {
        rubric_version: "v2".to_owned(),
        weights_version: "v2-polarity-fixed".to_owned(),
        weights: config.weights,
        resolved_model_id: response_model,
        state_hash,
        state_chars,
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

    use jev_driver::prelude::{JevClient, RetryConfig};
    use jev_driver::wire::{WireResponse, WireUsage};

    use super::{
        RESULTS_JSONL, RunSelector, RunnerConfig, RunnerMode, ScriptedTransport,
        assemble_arm_state, build_judge_work_list, build_summarizer, default_response, plan_run,
        run_bounded, run_concurrency, run_corpus, score_one, select_variants,
    };
    use crate::arms::{ArmConfig, SelectionArm};
    use crate::client::JevEndpoint;
    use crate::corpus::{CapturedEdge, Corpus, FailureClass, LabelledEdge, default_corpus};
    use crate::exhaustion::{CompletedKey, is_exhaustion};
    use crate::state::{PlanContext, PlanStep, WorkerOutput};

    /// Serializes tests that mutate process environment variables.
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

    #[test]
    fn summarizer_is_not_built_when_arm3_absent() {
        let summarizer = match build_summarizer(
            RunnerMode::Live,
            &[SelectionArm::HeadTruncation, SelectionArm::ClaimIndexed],
        ) {
            Ok(s) => s,
            Err(e) => panic!("arms 1 and 2 must not require summarizer env: {e}"),
        };
        assert!(
            summarizer.is_none(),
            "a run without Arm 3 must not construct a summarizer"
        );
    }

    #[tokio::test]
    async fn arm3_without_summarizer_env_fails_loud() {
        let _guard = ENV_LOCK.lock().await;

        let saved: Vec<(&str, Option<String>)> = [
            "SUMMARIZER_BASE_URL",
            "SUMMARIZER_API_KEY",
            "SUMMARIZER_MODEL",
        ]
        .into_iter()
        .map(|key| {
            let saved = std::env::var(key).ok();
            unsafe {
                std::env::remove_var(key);
            }
            (key, saved)
        })
        .collect();

        let construction = match build_summarizer(RunnerMode::Live, SelectionArm::all()) {
            Err(e) => e,
            Ok(_) => panic!("live Arm 3 without summarizer env must fail at construction"),
        };
        assert_eq!(
            construction.to_string(),
            "SUMMARIZER_BASE_URL is required for live Arm 3 (flash summarization)"
        );

        let edge = dummy_edge("v0", "result zero").edge;
        let assembly = match assemble_arm_state(
            &edge,
            SelectionArm::FlashSummarization,
            &ArmConfig::default(),
            None,
        )
        .await
        {
            Err(e) => e,
            Ok(_) => panic!("Arm 3 assembled without a summarizer must fail loud"),
        };
        assert_eq!(assembly.to_string(), construction.to_string());

        for (key, value) in saved {
            match value {
                Some(value) => unsafe {
                    std::env::set_var(key, value);
                },
                None => unsafe {
                    std::env::remove_var(key);
                },
            }
        }
    }

    fn dummy_edge(variant_id: &str, result: &str) -> LabelledEdge {
        LabelledEdge {
            variant_id: variant_id.to_owned(),
            class: FailureClass::Clean,
            edge: CapturedEdge {
                scenario: "test".to_owned(),
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
                    result: result.to_owned(),
                    truncated: false,
                },
                tool_calls: Vec::new(),
                confidence: None,
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

        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("temp dir: {e}"));

        let result = run_corpus(&corpus, &config, Some(dir.path()))
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

        let results_path = dir.path().join(RESULTS_JSONL);
        let persisted = std::fs::read_to_string(&results_path)
            .unwrap_or_else(|e| panic!("read results.jsonl: {e}"));
        assert_eq!(
            persisted.lines().count(),
            3,
            "three scoring records should be persisted"
        );

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

    #[test]
    fn judge_work_list_is_independent_of_this_run_scored() {
        let corpus = default_corpus().unwrap_or_else(|e| panic!("default corpus: {e}"));
        let completed: std::collections::HashSet<CompletedKey> = corpus
            .calibration
            .iter()
            .chain(corpus.held_out.iter())
            .flat_map(|edge| {
                SelectionArm::all()
                    .iter()
                    .map(move |&arm| CompletedKey::new(arm.name(), &edge.variant_id))
            })
            .collect();

        let arms = SelectionArm::all();
        let expected = corpus.held_out.len().saturating_mul(arms.len());
        let all = select_variants(&corpus, &RunSelector::All).unwrap_or_else(|e| panic!("{e}"));
        let work_list = build_judge_work_list(&corpus, &all, &completed, arms);
        assert_eq!(
            work_list.len(),
            expected,
            "judge work list must be held-out variants x arms when scoring is already done"
        );
        assert_eq!(
            work_list.len(),
            1716,
            "expected 572 held-out variants x 3 arms"
        );

        let smoke = select_variants(&corpus, &RunSelector::Smoke).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            build_judge_work_list(&corpus, &smoke, &completed, arms).is_empty(),
            "smoke draws from calibration, so it schedules no judge calls"
        );

        let slice =
            select_variants(&corpus, &RunSelector::Slice(5)).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            build_judge_work_list(&corpus, &slice, &completed, arms).is_empty(),
            "slice draws from calibration, so it schedules no judge calls"
        );

        let calibration_id = corpus.calibration[0].variant_id.clone();
        let calibration = select_variants(&corpus, &RunSelector::Edge(calibration_id))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(
            build_judge_work_list(&corpus, &calibration, &completed, arms).is_empty(),
            "a selected calibration edge is not held out"
        );

        let held_out_id = corpus.held_out[0].variant_id.clone();
        let held_out = select_variants(&corpus, &RunSelector::Edge(held_out_id))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            build_judge_work_list(&corpus, &held_out, &completed, arms).len(),
            arms.len(),
            "a selected held-out edge is judged once per arm"
        );

        let mut with_one_judge_done = completed;
        with_one_judge_done.insert(CompletedKey::judge(
            SelectionArm::ClaimIndexed.name(),
            &corpus.held_out[0].variant_id,
        ));
        let work_list = build_judge_work_list(&corpus, &all, &with_one_judge_done, arms);
        assert_eq!(
            work_list.len(),
            expected - 1,
            "already-completed judge key must be skipped"
        );
    }

    #[test]
    fn plan_run_judge_calls_match_judge_work_list() {
        let corpus = default_corpus().unwrap_or_else(|e| panic!("default corpus: {e}"));
        let held_out_id = corpus.held_out[0].variant_id.clone();
        let selectors = [
            RunSelector::Smoke,
            RunSelector::Slice(5),
            RunSelector::Edge(held_out_id),
            RunSelector::All,
        ];

        for selector in selectors {
            let selected = select_variants(&corpus, &selector).unwrap_or_else(|e| panic!("{e}"));
            let expected = build_judge_work_list(
                &corpus,
                &selected,
                &std::collections::HashSet::new(),
                SelectionArm::all(),
            )
            .len();
            let config = RunnerConfig {
                mode: RunnerMode::Live,
                selector,
                ..RunnerConfig::default()
            };
            let plan = plan_run(&corpus, &config).unwrap_or_else(|e| panic!("{e}"));

            assert_eq!(
                plan.judge_calls, expected,
                "dry-run judge_calls must match the live judge work list"
            );
        }
    }

    /// Every replay artifact must carry the acceptance-required provenance:
    /// a model id, a state hash, a state size, and token usage.
    #[tokio::test]
    async fn replay_artifacts_carry_provenance_fields() {
        let corpus = Corpus {
            calibration: vec![dummy_edge("prov-1", "result one")],
            held_out: Vec::new(),
        };
        let config = RunnerConfig::default();
        let result = run_corpus(&corpus, &config, None)
            .await
            .unwrap_or_else(|e| panic!("run_corpus: {e}"));

        assert_eq!(result.scored.len(), SelectionArm::all().len());
        for edge in &result.scored {
            let artifact = &edge.artifact;
            assert!(
                !artifact.resolved_model_id.is_empty(),
                "{} must record a resolved model id",
                edge.arm.name()
            );
            assert!(
                artifact.resolved_model_id.contains("replay"),
                "replay artifacts must carry the deterministic fixture id, got {}",
                artifact.resolved_model_id
            );
            assert_eq!(
                artifact.state_hash.len(),
                64,
                "{} must record a sha256 state hash",
                edge.arm.name()
            );
            assert!(
                artifact.state_chars > 0,
                "{} must record a non-zero state size",
                edge.arm.name()
            );
            assert!(
                artifact.tokens.input_tokens > 0,
                "{} must record input token usage",
                edge.arm.name()
            );
        }
    }

    /// Live-mode wiring: the resolved model id and token usage must flow from
    /// the API response, not the configured fallback. Driven through the
    /// scripted transport so the test never touches the network.
    #[tokio::test]
    async fn live_scripted_response_flows_model_and_usage_into_artifact() {
        let config = RunnerConfig::default();
        let edge = dummy_edge("live-1", "result one");
        let state = assemble_arm_state(
            &edge.edge,
            SelectionArm::ClaimIndexed,
            &config.arm_config,
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("assemble state: {e}"));

        let scripted = default_response();
        let transport = Arc::new(ScriptedTransport::new(vec![Ok(WireResponse {
            model: "resolved-live-model-42".to_owned(),
            answers: scripted.answers,
            usage: WireUsage {
                input_tokens: 4321,
                output_tokens: 99,
            },
        })]));
        let client = Arc::new(JevClient::with_transport(
            transport,
            "configured-fallback-model".to_owned(),
            RetryConfig::default(),
        ));
        let live_client = (client, "configured-fallback-model".to_owned());

        let scored = score_one(
            &edge,
            SelectionArm::ClaimIndexed,
            &state,
            &config,
            Some(&live_client),
        )
        .await
        .unwrap_or_else(|e| panic!("score_one: {e}"));

        let artifact = &scored.artifact;
        assert_eq!(
            artifact.resolved_model_id, "resolved-live-model-42",
            "live artifact must carry the API-resolved model id, not the configured fallback"
        );
        assert_eq!(artifact.tokens.input_tokens, 4321);
        assert_eq!(artifact.tokens.output_tokens, 99);
        assert_eq!(artifact.state_hash.len(), 64);
        assert!(artifact.state_chars > 0);
    }
}
